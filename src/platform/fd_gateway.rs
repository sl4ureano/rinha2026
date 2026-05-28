//! Epoll-based event-driven FD gateway: LB envia TCP FDs via SCM_RIGHTS;
//! single-threaded epoll loop processes all connections without spawning threads.
#![allow(static_mut_refs)]

use std::os::unix::io::RawFd;
use std::path::Path;
use std::sync::Arc;

use crate::http::response;
use crate::index::Index;
use crate::ingest::extract;
use crate::search::tier_fraud_count;

const MAX_FDS: usize = 65536;
const MAX_EVENTS: i32 = 512;
const BUF_CAP: usize = 8192;
const CTRL_LISTEN_TOKEN: u64 = u64::MAX;
const EPOLL_TIMEOUT_MS: i32 = 1; // 1ms like dalvorsn (reduces tail latency)

// Edge-triggered flags
const CLIENT_EVENTS: u32 = (libc::EPOLLIN | libc::EPOLLRDHUP | libc::EPOLLET) as u32;
const CLIENT_WRITE_EVENTS: u32 = (libc::EPOLLIN | libc::EPOLLOUT | libc::EPOLLRDHUP | libc::EPOLLET) as u32;
const CTRL_EVENTS: u32 = (libc::EPOLLIN | libc::EPOLLRDHUP | libc::EPOLLET) as u32;

// epoll busy-poll params (Linux 6.0+)
#[repr(C)]
struct EpollParams {
    busy_poll_usecs: u32,
    busy_poll_budget: u16,
    prefer_busy_poll: u8,
    _pad: u8,
}

const EPIOCSPARAMS: libc::c_ulong = 0x40087001;

struct Conn {
    buf: [u8; BUF_CAP],
    buf_len: usize,
    send_ptr: *const u8,
    send_len: usize,
    send_off: usize,
}

impl Conn {
    fn reset(&mut self) {
        self.buf_len = 0;
        self.send_ptr = std::ptr::null();
        self.send_len = 0;
        self.send_off = 0;
    }
}

static mut CONNS: [*mut Conn; MAX_FDS] = [std::ptr::null_mut(); MAX_FDS];
static mut IS_CTRL: [bool; MAX_FDS] = [false; MAX_FDS];
static mut EPFD: RawFd = -1;
static mut INDEX: *const Index = std::ptr::null();

#[inline]
unsafe fn get_conn(fd: RawFd) -> *mut Conn {
    let idx = fd as usize;
    if idx >= MAX_FDS {
        return std::ptr::null_mut();
    }
    if CONNS[idx].is_null() {
        let c = Box::into_raw(Box::new(Conn {
            buf: [0u8; BUF_CAP],
            buf_len: 0,
            send_ptr: std::ptr::null(),
            send_len: 0,
            send_off: 0,
        }));
        CONNS[idx] = c;
    }
    let c = CONNS[idx];
    (*c).reset();
    c
}

#[inline]
unsafe fn drop_conn(fd: RawFd) {
    libc::epoll_ctl(EPFD, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut());
    libc::close(fd);
}

pub fn run(index: Arc<Index>, sock_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(sock_path);

    unsafe {
        INDEX = Arc::into_raw(index);

        // mlockall for memory pinning
        libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE);

        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let ctrl_listen_fd = create_unix_listener(sock_path)?;
    eprintln!("listening on uds fd-passing {}", sock_path.display());

    unsafe {
        EPFD = libc::epoll_create1(libc::EPOLL_CLOEXEC);
        if EPFD < 0 {
            return Err(anyhow::anyhow!("epoll_create1: {}", std::io::Error::last_os_error()));
        }

        // Configure epoll busy-poll
        let params = EpollParams {
            busy_poll_usecs: 50,
            busy_poll_budget: 8,
            prefer_busy_poll: 1,
            _pad: 0,
        };
        let ret = libc::ioctl(EPFD, EPIOCSPARAMS, &params as *const EpollParams);
        if ret < 0 {
            eprintln!("EPIOCSPARAMS: {} (non-fatal)", std::io::Error::last_os_error());
        } else {
            eprintln!("epoll busy_poll=50us budget=8 prefer=1");
        }

        // Register ctrl listener (edge-triggered)
        let mut ev = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLET) as u32,
            u64: CTRL_LISTEN_TOKEN,
        };
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, ctrl_listen_fd, &mut ev);

        event_loop(ctrl_listen_fd);
    }
}

unsafe fn event_loop(ctrl_listen_fd: RawFd) -> ! {
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS as usize];

    loop {
        let nfds = libc::epoll_wait(EPFD, events.as_mut_ptr(), MAX_EVENTS, EPOLL_TIMEOUT_MS);
        if nfds < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            continue;
        }

        for i in 0..nfds as usize {
            let token = events[i].u64;
            let revents = events[i].events;

            if token == CTRL_LISTEN_TOKEN {
                accept_ctrl_conn(ctrl_listen_fd);
                continue;
            }

            let fd = token as RawFd;

            if (fd as usize) < MAX_FDS && IS_CTRL[fd as usize] {
                if revents & (libc::EPOLLHUP | libc::EPOLLERR | libc::EPOLLRDHUP) as u32 != 0 {
                    libc::epoll_ctl(EPFD, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut());
                    IS_CTRL[fd as usize] = false;
                    libc::close(fd);
                } else if revents & libc::EPOLLIN as u32 != 0 {
                    accept_from_lb(fd);
                }
                continue;
            }

            if revents & (libc::EPOLLHUP | libc::EPOLLERR) as u32 != 0 {
                drop_conn(fd);
                continue;
            }
            if revents & libc::EPOLLIN as u32 != 0 {
                handle_client_read(fd);
            }
            if revents & libc::EPOLLOUT as u32 != 0 {
                handle_client_write(fd);
            }
            if revents & libc::EPOLLRDHUP as u32 != 0 {
                drop_conn(fd);
            }
        }
    }
}

unsafe fn accept_ctrl_conn(ctrl_listen_fd: RawFd) {
    loop {
        let cfd = libc::accept4(ctrl_listen_fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC);
        if cfd < 0 {
            return;
        }
        if (cfd as usize) >= MAX_FDS {
            libc::close(cfd);
            continue;
        }
        IS_CTRL[cfd as usize] = true;
        let mut ev = libc::epoll_event {
            events: CTRL_EVENTS,
            u64: cfd as u64,
        };
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, cfd, &mut ev);
    }
}

unsafe fn accept_from_lb(ctrl: RawFd) {
    for _ in 0..64 {
        let client_fd = recv_fd(ctrl);
        if client_fd < 0 {
            return;
        }
        if (client_fd as usize) >= MAX_FDS {
            libc::close(client_fd);
            continue;
        }

        // Set non-blocking
        let flags = libc::fcntl(client_fd, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(client_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        // TCP_NODELAY + TCP_QUICKACK
        let one: libc::c_int = 1;
        libc::setsockopt(client_fd, libc::IPPROTO_TCP, libc::TCP_NODELAY, &one as *const _ as *const _, 4);
        libc::setsockopt(client_fd, libc::IPPROTO_TCP, libc::TCP_QUICKACK, &one as *const _ as *const _, 4);

        let c = get_conn(client_fd);
        if c.is_null() {
            libc::close(client_fd);
            continue;
        }

        // Greedy read: try to read immediately before registering with epoll
        let n = libc::recv(client_fd, (*c).buf.as_mut_ptr() as *mut _, BUF_CAP, 0);
        if n > 0 {
            (*c).buf_len = n as usize;
            // Try to process immediately
            if try_process_request(client_fd, c) {
                continue; // Fully handled inline
            }
        } else if n == 0 {
            libc::close(client_fd);
            continue;
        }
        // n < 0 means EAGAIN, register normally (edge-triggered)

        let mut ev = libc::epoll_event {
            events: CLIENT_EVENTS,
            u64: client_fd as u64,
        };
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, client_fd, &mut ev);
    }
}

/// Returns true if connection is fully handled (response sent and closed/re-armed)
unsafe fn try_process_request(fd: RawFd, c: *mut Conn) -> bool {
    loop {
        let buf = &(*c).buf[..(*c).buf_len];
        let header_end = match find_double_crlf(buf) {
            Some(p) => p,
            None => return false,
        };

        // Fast path for POST /fraud-score
        if buf.len() >= 21 && &buf[..5] == b"POST " {
            let cl = match content_length_fast(&buf[..header_end]) {
                Some(cl) => cl,
                None => {
                    send_response_inline(fd, response::RESP_DENIED_S10);
                    return true;
                }
            };
            if buf.len() < header_end + cl {
                return false; // Need more data
            }
            let body = &buf[header_end..header_end + cl];
            let resp = fraud_response(body);
            let consumed = header_end + cl;

            // Try to send inline
            let sent = libc::send(fd, resp.as_ptr() as *const _, resp.len(), libc::MSG_NOSIGNAL);
            if sent as usize == resp.len() {
                // Consume from buffer
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy((*c).buf.as_ptr().add(consumed), (*c).buf.as_mut_ptr(), leftover);
                    (*c).buf_len = leftover;
                    continue; // Try to process next pipelined request
                }
                (*c).buf_len = 0;
                // ET: register for next request (keep-alive)
                let mut ev = libc::epoll_event {
                    events: CLIENT_EVENTS,
                    u64: fd as u64,
                };
                libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, fd, &mut ev);
                return true;
            } else if sent < 0 {
                let err = *libc::__errno_location();
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    (*c).send_ptr = resp.as_ptr();
                    (*c).send_len = resp.len();
                    (*c).send_off = 0;
                    let leftover = (*c).buf_len - consumed;
                    if leftover > 0 {
                        std::ptr::copy((*c).buf.as_ptr().add(consumed), (*c).buf.as_mut_ptr(), leftover);
                    }
                    (*c).buf_len = leftover;
                    let mut ev = libc::epoll_event {
                        events: CLIENT_WRITE_EVENTS,
                        u64: fd as u64,
                    };
                    libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, fd, &mut ev);
                    return true;
                }
                libc::close(fd);
                return true;
            } else {
                // Partial send
                (*c).send_ptr = resp.as_ptr();
                (*c).send_len = resp.len();
                (*c).send_off = sent as usize;
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy((*c).buf.as_ptr().add(consumed), (*c).buf.as_mut_ptr(), leftover);
                }
                (*c).buf_len = leftover;
                let mut ev = libc::epoll_event {
                    events: CLIENT_WRITE_EVENTS,
                    u64: fd as u64,
                };
                libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, fd, &mut ev);
                return true;
            }
        }

        // GET /ready
        if buf.len() >= 10 && &buf[..3] == b"GET" {
            send_response_inline(fd, response::RESP_READY);
            let mut ev = libc::epoll_event {
                events: (libc::EPOLLIN | libc::EPOLLRDHUP) as u32,
                u64: fd as u64,
            };
            libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, fd, &mut ev);
            return true;
        }

        send_response_inline(fd, response::RESP_NOT_FOUND);
        libc::close(fd);
        return true;
    }
}

unsafe fn handle_client_read(fd: RawFd) {
    let idx = fd as usize;
    if idx >= MAX_FDS || CONNS[idx].is_null() {
        drop_conn(fd);
        return;
    }
    let c = CONNS[idx];

    // ET mode: drain the socket fully
    loop {
        let room = BUF_CAP - (*c).buf_len;
        if room == 0 {
            break;
        }
        let n = libc::recv(fd, (*c).buf.as_mut_ptr().add((*c).buf_len) as *mut _, room, 0);
        if n > 0 {
            (*c).buf_len += n as usize;
        } else if n == 0 {
            drop_conn(fd);
            return;
        } else {
            let err = *libc::__errno_location();
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                break;
            }
            drop_conn(fd);
            return;
        }
    }

    // Process pipelined requests
    loop {
        let buf = &(*c).buf[..(*c).buf_len];
        let header_end = match find_double_crlf(buf) {
            Some(p) => p,
            None => return,
        };

        if buf.len() >= 5 && &buf[..5] == b"POST " {
            let cl = match content_length_fast(&buf[..header_end]) {
                Some(cl) => cl,
                None => {
                    send_and_close(fd, response::RESP_DENIED_S10);
                    return;
                }
            };
            if buf.len() < header_end + cl {
                return; // Need more body
            }
            let body = &buf[header_end..header_end + cl];
            let resp = fraud_response(body);
            let consumed = header_end + cl;

            let sent = libc::send(fd, resp.as_ptr() as *const _, resp.len(), libc::MSG_NOSIGNAL);
            if sent as usize == resp.len() {
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy((*c).buf.as_ptr().add(consumed), (*c).buf.as_mut_ptr(), leftover);
                    (*c).buf_len = leftover;
                    continue; // Pipeline
                }
                (*c).buf_len = 0;
                return;
            } else if sent < 0 {
                let err = *libc::__errno_location();
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    (*c).send_ptr = resp.as_ptr();
                    (*c).send_len = resp.len();
                    (*c).send_off = 0;
                    let leftover = (*c).buf_len - consumed;
                    if leftover > 0 {
                        std::ptr::copy((*c).buf.as_ptr().add(consumed), (*c).buf.as_mut_ptr(), leftover);
                    }
                    (*c).buf_len = leftover;
                    let mut ev = libc::epoll_event {
                        events: CLIENT_WRITE_EVENTS,
                        u64: fd as u64,
                    };
                    libc::epoll_ctl(EPFD, libc::EPOLL_CTL_MOD, fd, &mut ev);
                    return;
                }
                drop_conn(fd);
                return;
            } else {
                // Partial send
                (*c).send_ptr = resp.as_ptr();
                (*c).send_len = resp.len();
                (*c).send_off = sent as usize;
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy((*c).buf.as_ptr().add(consumed), (*c).buf.as_mut_ptr(), leftover);
                }
                (*c).buf_len = leftover;
                let mut ev = libc::epoll_event {
                    events: CLIENT_WRITE_EVENTS,
                    u64: fd as u64,
                };
                libc::epoll_ctl(EPFD, libc::EPOLL_CTL_MOD, fd, &mut ev);
                return;
            }
        } else if buf.len() >= 3 && &buf[..3] == b"GET" {
            let resp = response::RESP_READY;
            let _ = libc::send(fd, resp.as_ptr() as *const _, resp.len(), libc::MSG_NOSIGNAL);
            let leftover = (*c).buf_len - header_end;
            if leftover > 0 {
                std::ptr::copy((*c).buf.as_ptr().add(header_end), (*c).buf.as_mut_ptr(), leftover);
                (*c).buf_len = leftover;
                continue;
            }
            (*c).buf_len = 0;
            return;
        } else {
            send_and_close(fd, response::RESP_NOT_FOUND);
            return;
        }
    }
}

unsafe fn handle_client_write(fd: RawFd) {
    let idx = fd as usize;
    if idx >= MAX_FDS || CONNS[idx].is_null() {
        drop_conn(fd);
        return;
    }
    let c = CONNS[idx];
    if (*c).send_ptr.is_null() {
        let mut ev = libc::epoll_event {
            events: CLIENT_EVENTS,
            u64: fd as u64,
        };
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_MOD, fd, &mut ev);
        return;
    }

    // ET mode: drain send buffer fully
    loop {
        let remaining = (*c).send_len - (*c).send_off;
        if remaining == 0 {
            break;
        }
        let n = libc::send(fd, (*c).send_ptr.add((*c).send_off) as *const _, remaining, libc::MSG_NOSIGNAL);
        if n > 0 {
            (*c).send_off += n as usize;
        } else {
            let err = *libc::__errno_location();
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                return; // Still need EPOLLOUT, ET will re-fire
            }
            drop_conn(fd);
            return;
        }
    }

    // Done sending, switch back to read-only ET
    (*c).send_ptr = std::ptr::null();
    (*c).send_off = 0;
    (*c).send_len = 0;
    let mut ev = libc::epoll_event {
        events: CLIENT_EVENTS,
        u64: fd as u64,
    };
    libc::epoll_ctl(EPFD, libc::EPOLL_CTL_MOD, fd, &mut ev);
}

#[inline]
unsafe fn send_response_inline(fd: RawFd, resp: &[u8]) {
    let _ = libc::send(fd, resp.as_ptr() as *const _, resp.len(), libc::MSG_NOSIGNAL);
}

#[inline]
unsafe fn send_and_close(fd: RawFd, resp: &[u8]) {
    let _ = libc::send(fd, resp.as_ptr() as *const _, resp.len(), libc::MSG_NOSIGNAL);
    drop_conn(fd);
}

#[inline]
fn fraud_response(body: &[u8]) -> &'static [u8] {
    match extract(body) {
        Some(p) => response::for_count(tier_fraud_count(&p)),
        None => response::RESP_DENIED_S10,
    }
}

fn create_unix_listener(path: &Path) -> anyhow::Result<RawFd> {
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.len() >= addr.sun_path.len() {
        unsafe { libc::close(fd) };
        anyhow::bail!("uds path too long");
    }
    for (i, &b) in bytes.iter().enumerate() {
        addr.sun_path[i] = b as libc::c_char;
    }
    let r = unsafe {
        libc::bind(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e.into());
    }
    unsafe {
        let cpath = std::ffi::CString::new(bytes)?;
        libc::chmod(cpath.as_ptr(), 0o666);
    }
    let r = unsafe { libc::listen(fd, 1024) };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e.into());
    }
    Ok(fd)
}

unsafe fn recv_fd(control_fd: RawFd) -> RawFd {
    let mut one: u8 = 0;
    let mut iov = libc::iovec {
        iov_base: &mut one as *mut _ as *mut _,
        iov_len: 1,
    };
    let mut cmsg_buf = [0u8; 64];
    let mut msg: libc::msghdr = std::mem::zeroed();
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut _;
    msg.msg_controllen = cmsg_buf.len();

    let n = libc::recvmsg(control_fd, &mut msg, libc::MSG_DONTWAIT);
    if n <= 0 {
        return -1;
    }
    let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
    while !cmsg.is_null() {
        if (*cmsg).cmsg_level == libc::SOL_SOCKET
            && (*cmsg).cmsg_type == libc::SCM_RIGHTS
            && (*cmsg).cmsg_len >= libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as u32) as _
        {
            let mut fd: libc::c_int = -1;
            std::ptr::copy_nonoverlapping(
                libc::CMSG_DATA(cmsg) as *const u8,
                &mut fd as *mut libc::c_int as *mut u8,
                std::mem::size_of::<libc::c_int>(),
            );
            return fd;
        }
        cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
    }
    -1
}

#[inline]
fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    let mut i = 0;
    while i + 4 <= buf.len() {
        let w = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        if w == 0x0a0d_0a0d {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}

#[inline]
fn content_length_fast(headers: &[u8]) -> Option<usize> {
    const TAG: &[u8] = b"\r\nContent-Length: ";
    let mut i = 0;
    while i + TAG.len() <= headers.len() {
        if &headers[i..i + TAG.len()] == TAG {
            let start = i + TAG.len();
            let mut end = start;
            while end < headers.len() && headers[end].is_ascii_digit() {
                end += 1;
            }
            if end > start {
                let mut n = 0usize;
                for &b in &headers[start..end] {
                    n = n * 10 + (b - b'0') as usize;
                }
                return Some(n);
            }
        }
        i += 1;
    }
    None
}