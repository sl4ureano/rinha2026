//! Epoll-based event-driven FD gateway: LB envia TCP FDs via SCM_RIGHTS;
//! single-threaded epoll loop processes all connections without spawning threads.
#![allow(static_mut_refs)]

use std::os::unix::io::RawFd;
use std::path::Path;

use std::sync::Arc;

use crate::http::response;
use crate::index::Index;
use crate::ingest::extract;
use crate::perf;
use crate::search::{complete_cache, run_warmup, tier_gray_count, try_fast_fraud_count};

/// Limite de fds rastreados (8k cobre a prova; evita ~512 KiB de tabela estática).
const MAX_FDS: usize = 8192;
const MAX_EVENTS: i32 = 512;
const BUF_CAP: usize = 8192;
const CONN_POOL_CAP: usize = 256;
const CTRL_LISTEN_TOKEN: u64 = u64::MAX;
const TCP_LISTEN_TOKEN: u64 = u64::MAX - 1;
const HEALTH_LISTEN_TOKEN: u64 = u64::MAX - 2;
const EPOLL_TIMEOUT_MS: i32 = 1;

// Edge-triggered flags
const CLIENT_EVENTS: u32 = (libc::EPOLLIN | libc::EPOLLRDHUP | libc::EPOLLET) as u32;
const CLIENT_WRITE_EVENTS: u32 =
    (libc::EPOLLIN | libc::EPOLLOUT | libc::EPOLLRDHUP | libc::EPOLLET) as u32;
const CTRL_EVENTS: u32 = (libc::EPOLLIN | libc::EPOLLRDHUP | libc::EPOLLET) as u32;

// epoll busy-poll params (Linux 6.9+)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct EpollParams {
    busy_poll_usecs: u32,
    busy_poll_budget: u16,
    prefer_busy_poll: u8,
    _pad: u8,
}

const EPIOCSPARAMS: libc::c_ulong = 0x40088A01;
const EPIOCGPARAMS: libc::c_ulong = 0x80088A02;

struct Conn {
    buf: [u8; BUF_CAP],
    buf_len: usize,
    send_ptr: *const u8,
    send_len: usize,
    send_off: usize,
    request_start: Option<std::time::Instant>,
    pending_request_start: Option<std::time::Instant>,
    pending_write_start: Option<std::time::Instant>,
    pending_success: bool,
}

impl Conn {
    fn reset(&mut self) {
        self.buf_len = 0;
        self.send_ptr = std::ptr::null();
        self.send_len = 0;
        self.send_off = 0;
        self.request_start = None;
        self.pending_request_start = None;
        self.pending_write_start = None;
        self.pending_success = false;
    }
}

static mut CONNS: [*mut Conn; MAX_FDS] = [std::ptr::null_mut(); MAX_FDS];
static mut IS_CTRL: [bool; MAX_FDS] = [false; MAX_FDS];
static mut CONN_POOL: [*mut Conn; CONN_POOL_CAP] = [std::ptr::null_mut(); CONN_POOL_CAP];
static mut CONN_POOL_LEN: usize = 0;
static mut EPFD: RawFd = -1;
static mut INDEX_PTR: *const Index = std::ptr::null();

fn env_us(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

unsafe fn pool_pop() -> Option<*mut Conn> {
    if CONN_POOL_LEN == 0 {
        return None;
    }
    CONN_POOL_LEN -= 1;
    Some(CONN_POOL[CONN_POOL_LEN])
}

unsafe fn pool_push(c: *mut Conn) {
    if CONN_POOL_LEN < CONN_POOL_CAP {
        CONN_POOL[CONN_POOL_LEN] = c;
        CONN_POOL_LEN += 1;
    } else {
        drop(Box::from_raw(c));
    }
}

fn configure_epoll_params() {
    let requested = env_truthy("EPOLL_BUSY_POLL");
    if !requested {
        perf::set_epoll_busy_poll_result(false, false, 0, 0, 0, 0, 0);
        eprintln!("epoll busy_poll=disabled timeout={EPOLL_TIMEOUT_MS}ms");
        return;
    }

    let profile = std::env::var("EPOLL_BUSY_POLL_PROFILE")
        .unwrap_or_else(|_| "B".to_string())
        .to_ascii_uppercase();
    let (default_usecs, default_budget) = match profile.as_str() {
        "A" => (10, 4),
        "B" => (25, 8),
        "C" => (50, 16),
        "D" => (100, 32),
        _ => (25, 8),
    };
    let busy_poll = env_us("EPOLL_BUSY_POLL_US", default_usecs);
    let budget = env_us("EPOLL_BUSY_POLL_BUDGET", default_budget) as u16;
    unsafe {
        let params = EpollParams {
            busy_poll_usecs: busy_poll,
            busy_poll_budget: budget,
            prefer_busy_poll: 1,
            _pad: 0,
        };
        let ret = libc::ioctl(EPFD, EPIOCSPARAMS, &params as *const EpollParams);
        if ret < 0 {
            let errno = *libc::__errno_location();
            perf::set_epoll_busy_poll_result(false, false, 0, 0, 0, errno, 0);
            eprintln!(
                "EPIOCSPARAMS unsupported, continuing without epoll busy poll: {}",
                std::io::Error::from_raw_os_error(errno)
            );
            return;
        }

        let mut actual = EpollParams::default();
        let get_ret = libc::ioctl(EPFD, EPIOCGPARAMS, &mut actual as *mut EpollParams);
        if get_ret < 0 {
            let errno = *libc::__errno_location();
            perf::set_epoll_busy_poll_result(false, false, 0, 0, 0, 0, errno);
            eprintln!(
                "EPIOCGPARAMS unsupported, continuing without epoll busy poll validation: {}",
                std::io::Error::from_raw_os_error(errno)
            );
            return;
        }

        let applied = actual.busy_poll_usecs == busy_poll
            && actual.busy_poll_budget == budget
            && actual.prefer_busy_poll == 1;
        perf::set_epoll_busy_poll_result(
            applied,
            applied,
            actual.busy_poll_usecs,
            actual.busy_poll_budget,
            actual.prefer_busy_poll,
            0,
            0,
        );
        if !applied {
            eprintln!(
                "kernel ignored EPIOCSPARAMS, continuing without epoll busy poll: requested usecs={busy_poll} budget={budget} prefer=1, got usecs={} budget={} prefer={}",
                actual.busy_poll_usecs, actual.busy_poll_budget, actual.prefer_busy_poll
            );
            return;
        }
        eprintln!(
            "epoll busy_poll={}us budget={} prefer={} timeout={}ms",
            actual.busy_poll_usecs,
            actual.busy_poll_budget,
            actual.prefer_busy_poll,
            EPOLL_TIMEOUT_MS
        );
    }
}

/// Re-arm client fd for epoll (MOD if already registered, else ADD).
#[inline]
unsafe fn epoll_arm(fd: RawFd, events: u32) {
    let mut ev = libc::epoll_event {
        events,
        u64: fd as u64,
    };
    if libc::epoll_ctl(EPFD, libc::EPOLL_CTL_MOD, fd, &mut ev) < 0 {
        let err = *libc::__errno_location();
        if err == libc::ENOENT {
            libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, fd, &mut ev);
        }
    }
}

#[inline]
unsafe fn get_conn(fd: RawFd) -> *mut Conn {
    let idx = fd as usize;
    if idx >= MAX_FDS {
        return std::ptr::null_mut();
    }
    if CONNS[idx].is_null() {
        let c = pool_pop().unwrap_or_else(|| {
            Box::into_raw(Box::new(Conn {
                buf: [0u8; BUF_CAP],
                buf_len: 0,
                send_ptr: std::ptr::null(),
                send_len: 0,
                send_off: 0,
                request_start: None,
                pending_request_start: None,
                pending_write_start: None,
                pending_success: false,
            }))
        });
        CONNS[idx] = c;
        perf::connection_opened();
    }
    let c = CONNS[idx];
    (*c).reset();
    c
}

#[inline]
unsafe fn drop_conn(fd: RawFd) {
    let idx = fd as usize;
    if idx < MAX_FDS && !CONNS[idx].is_null() {
        pool_push(CONNS[idx]);
        CONNS[idx] = std::ptr::null_mut();
        perf::connection_closed();
    }
    libc::epoll_ctl(EPFD, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut());
    libc::close(fd);
}

#[inline]
unsafe fn socket_write(fd: RawFd, ptr: *const u8, len: usize) -> isize {
    libc::write(fd, ptr as *const _, len) as isize
}

/// Direct TCP mode: server listens on TCP port directly (no LB, no fd-passing).
pub fn run_direct(index: Arc<Index>, port: u16) -> anyhow::Result<()> {
    unsafe {
        INDEX_PTR = Arc::into_raw(index);
        libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let tcp_fd = create_tcp_listener(port)?;
    eprintln!("listening tcp-direct :{port} (epoll ET)");

    unsafe {
        EPFD = libc::epoll_create1(libc::EPOLL_CLOEXEC);
        if EPFD < 0 {
            return Err(anyhow::anyhow!(
                "epoll_create1: {}",
                std::io::Error::last_os_error()
            ));
        }

        configure_epoll_params();

        let mut ev = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLET) as u32,
            u64: TCP_LISTEN_TOKEN,
        };
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, tcp_fd, &mut ev);

        run_warmup();
        crate::perf::reset();
        event_loop_direct(tcp_fd);
    }
}

/// Direct TCP event loop — accepts connections and processes inline.
unsafe fn event_loop_direct(tcp_fd: RawFd) -> ! {
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS as usize];

    loop {
        let wait_start = perf::stage_start();
        let nfds = libc::epoll_wait(EPFD, events.as_mut_ptr(), MAX_EVENTS, EPOLL_TIMEOUT_MS);
        perf::record_epoll_wait(wait_start, nfds);
        if nfds < 0 {
            continue;
        }

        for i in 0..nfds as usize {
            let dispatch_start = perf::stage_start();
            let token = events[i].u64;
            let revents = events[i].events;

            if token == TCP_LISTEN_TOKEN {
                perf::record_epoll_dispatch(dispatch_start);
                accept_tcp_clients(tcp_fd);
                continue;
            }

            let fd = token as RawFd;

            if revents & (libc::EPOLLHUP | libc::EPOLLERR) as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                drop_conn(fd);
                continue;
            }
            if revents & libc::EPOLLIN as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                handle_client_read(fd);
            }
            if revents & libc::EPOLLOUT as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                handle_client_write(fd);
            }
            if revents & libc::EPOLLRDHUP as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                drop_conn(fd);
            }
        }
    }
}

unsafe fn accept_tcp_clients(tcp_fd: RawFd) {
    loop {
        let client_fd = libc::accept4(
            tcp_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
        );
        if client_fd < 0 {
            return;
        }
        if (client_fd as usize) >= MAX_FDS {
            libc::close(client_fd);
            continue;
        }

        let one: libc::c_int = 1;
        libc::setsockopt(
            client_fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &one as *const _ as *const _,
            4,
        );
        libc::setsockopt(
            client_fd,
            libc::IPPROTO_TCP,
            libc::TCP_QUICKACK,
            &one as *const _ as *const _,
            4,
        );

        let c = get_conn(client_fd);
        if c.is_null() {
            libc::close(client_fd);
            continue;
        }
        (*c).request_start = perf::stage_start();

        // Greedy read: try to read and process inline
        let recv_start = perf::stage_start();
        perf::recv_call();
        let n = libc::recv(client_fd, (*c).buf.as_mut_ptr() as *mut _, BUF_CAP, 0);
        perf::record_stage(perf::STAGE_SOCKET_RECV, recv_start);
        if n > 0 {
            (*c).buf_len = n as usize;
            perf::add_bytes_received(n as usize);
            if try_process_request(client_fd, c) {
                continue;
            }
        } else if n == 0 {
            drop_conn(client_fd);
            continue;
        }

        let mut ev = libc::epoll_event {
            events: CLIENT_EVENTS,
            u64: client_fd as u64,
        };
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, client_fd, &mut ev);
    }
}

fn create_tcp_listener(port: u16) -> anyhow::Result<RawFd> {
    let fd = unsafe {
        libc::socket(
            libc::AF_INET,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    unsafe {
        let one: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &one as *const _ as *const _,
            4,
        );
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &one as *const _ as *const _,
            4,
        );
        libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &one as *const _ as *const _,
            4,
        );
        let defer: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_DEFER_ACCEPT,
            &defer as *const _ as *const _,
            4,
        );
        let tfo: libc::c_int = 5;
        libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_FASTOPEN,
            &tfo as *const _ as *const _,
            4,
        );
    }
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as libc::sa_family_t;
    addr.sin_addr.s_addr = u32::to_be(libc::INADDR_ANY);
    addr.sin_port = port.to_be();
    if unsafe {
        libc::bind(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    } != 0
    {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e.into());
    }
    if unsafe { libc::listen(fd, 65535) } != 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e.into());
    }
    Ok(fd)
}

pub fn run(sock_path: &Path, index: Arc<Index>, health_port: u16) -> anyhow::Result<()> {
    unsafe {
        INDEX_PTR = Arc::into_raw(index);
    }
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(sock_path);

    unsafe {
        libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let ctrl_listen_fd = create_unix_listener(sock_path)?;
    let health_fd = create_tcp_listener(health_port)?;
    eprintln!(
        "listening on uds fd-passing {} + health :{health_port}",
        sock_path.display()
    );

    unsafe {
        EPFD = libc::epoll_create1(libc::EPOLL_CLOEXEC);
        if EPFD < 0 {
            return Err(anyhow::anyhow!(
                "epoll_create1: {}",
                std::io::Error::last_os_error()
            ));
        }

        configure_epoll_params();

        let mut ev = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLET) as u32,
            u64: CTRL_LISTEN_TOKEN,
        };
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, ctrl_listen_fd, &mut ev);

        ev.u64 = HEALTH_LISTEN_TOKEN;
        libc::epoll_ctl(EPFD, libc::EPOLL_CTL_ADD, health_fd, &mut ev);

        run_warmup();
        crate::perf::reset();
        event_loop(ctrl_listen_fd, health_fd);
    }
}

unsafe fn event_loop(ctrl_listen_fd: RawFd, health_fd: RawFd) -> ! {
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS as usize];

    loop {
        let wait_start = perf::stage_start();
        let nfds = libc::epoll_wait(EPFD, events.as_mut_ptr(), MAX_EVENTS, EPOLL_TIMEOUT_MS);
        perf::record_epoll_wait(wait_start, nfds);
        if nfds < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            continue;
        }

        for i in 0..nfds as usize {
            let dispatch_start = perf::stage_start();
            let token = events[i].u64;
            let revents = events[i].events;

            if token == CTRL_LISTEN_TOKEN {
                perf::record_epoll_dispatch(dispatch_start);
                accept_ctrl_conn(ctrl_listen_fd);
                continue;
            }
            if token == HEALTH_LISTEN_TOKEN {
                perf::record_epoll_dispatch(dispatch_start);
                accept_health_clients(health_fd);
                continue;
            }

            let fd = token as RawFd;

            if (fd as usize) < MAX_FDS && IS_CTRL[fd as usize] {
                if revents & (libc::EPOLLHUP | libc::EPOLLERR | libc::EPOLLRDHUP) as u32 != 0 {
                    perf::record_epoll_dispatch(dispatch_start);
                    libc::epoll_ctl(EPFD, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut());
                    IS_CTRL[fd as usize] = false;
                    libc::close(fd);
                } else if revents & libc::EPOLLIN as u32 != 0 {
                    perf::record_epoll_dispatch(dispatch_start);
                    accept_from_lb(fd);
                }
                continue;
            }

            if revents & (libc::EPOLLHUP | libc::EPOLLERR) as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                drop_conn(fd);
                continue;
            }
            if revents & libc::EPOLLIN as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                handle_client_read(fd);
            }
            if revents & libc::EPOLLOUT as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                handle_client_write(fd);
            }
            if revents & libc::EPOLLRDHUP as u32 != 0 {
                perf::record_epoll_dispatch(dispatch_start);
                drop_conn(fd);
            }
        }
    }
}

unsafe fn accept_health_clients(health_fd: RawFd) {
    loop {
        let client_fd = libc::accept4(
            health_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
        );
        if client_fd < 0 {
            return;
        }
        let mut buf = [0u8; 256];
        let n = libc::recv(client_fd, buf.as_mut_ptr() as *mut _, buf.len(), 0);
        if n > 0 && buf[..n as usize].windows(6).any(|w| w == b"/ready") {
            let _ = socket_write(
                client_fd,
                response::RESP_READY.as_ptr(),
                response::RESP_READY.len(),
            );
        }
        libc::close(client_fd);
    }
}

unsafe fn accept_ctrl_conn(ctrl_listen_fd: RawFd) {
    loop {
        let cfd = libc::accept4(
            ctrl_listen_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
        );
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
        let recv_fd_start = perf::stage_start();
        let client_fd = recv_fd(ctrl);
        if client_fd < 0 {
            return;
        }
        perf::record_stage(perf::STAGE_API_RECV_FD, recv_fd_start);
        perf::recv_fd_ok();
        if (client_fd as usize) >= MAX_FDS {
            libc::close(client_fd);
            continue;
        }

        // Set socket options here (LB sends bare fd for minimum overhead)
        let one: libc::c_int = 1;
        let sockopt_start = perf::stage_start();
        libc::setsockopt(
            client_fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &one as *const _ as *const _,
            4,
        );
        libc::setsockopt(
            client_fd,
            libc::IPPROTO_TCP,
            libc::TCP_QUICKACK,
            &one as *const _ as *const _,
            4,
        );
        perf::record_stage(perf::STAGE_API_SETSOCKOPT, sockopt_start);

        let c = get_conn(client_fd);
        if c.is_null() {
            libc::close(client_fd);
            continue;
        }
        (*c).request_start = perf::stage_start();

        // Spin-read: try recv up to 32 times before falling back to epoll.
        // With TCP_DEFER_ACCEPT on the LB, data is usually already in the
        // kernel buffer — but timing jitter from fd-passing may delay it by
        // a few microseconds. Spinning here avoids a costly epoll round-trip.
        let mut got_data = false;
        let mut attempts = 0u64;
        let spin_start = perf::stage_start();
        for _ in 0..32 {
            attempts += 1;
            perf::recv_call();
            let n = libc::recv(client_fd, (*c).buf.as_mut_ptr() as *mut _, BUF_CAP, 0);
            if n > 0 {
                (*c).buf_len = n as usize;
                perf::add_bytes_received(n as usize);
                got_data = true;
                break;
            } else if n == 0 {
                drop_conn(client_fd);
                got_data = true; // signal to skip epoll registration
                break;
            }
            // EAGAIN — data not yet available, spin briefly
            std::hint::spin_loop();
        }
        perf::record_stage(perf::STAGE_SPIN_READ, spin_start);
        perf::spin_read_result(got_data && (*c).buf_len > 0, attempts);

        if got_data && (*c).buf_len > 0 {
            if try_process_request(client_fd, c) {
                continue;
            }
        } else if got_data {
            continue; // was closed (n==0)
        }

        // Fall back to epoll if spin-read didn't get data
        perf::epoll_read_fallback();
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
        let processing_start = perf::stage_start();
        let validation_start = perf::stage_start();
        let http_start = perf::stage_start();
        let buf = &(&(*c).buf)[..(*c).buf_len];
        let header_end = match find_double_crlf(buf) {
            Some(p) => p,
            None => {
                perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
                return false;
            }
        };

        // Fast path for POST /fraud-score
        if buf.len() >= 21 && &buf[..5] == b"POST " {
            let cl = match content_length_fast(&buf[..header_end]) {
                Some(cl) => cl,
                None => {
                    perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
                    perf::record_stage(perf::STAGE_VALIDATION, validation_start);
                    send_response_inline(fd, response::RESP_DENIED_S10);
                    finish_request(c, current_request_start(c, processing_start), false);
                    return true;
                }
            };
            perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
            if buf.len() < header_end + cl {
                return false; // Need more data
            }
            perf::record_stage(perf::STAGE_VALIDATION, validation_start);
            let body = &buf[header_end..header_end + cl];
            let (resp, success) = fraud_response(body);
            let consumed = header_end + cl;
            let total_start = current_request_start(c, processing_start);

            // Try to send inline
            let write_start = perf::stage_start();
            perf::send_call();
            let sent = socket_write(fd, resp.as_ptr(), resp.len());
            perf::record_stage(perf::STAGE_WRITE_RESPONSE, write_start);
            if sent as usize == resp.len() {
                perf::add_bytes_sent(sent as usize);
                perf::record_stage(perf::STAGE_WRITE_COMPLETE, write_start);
                finish_request(c, total_start, success);
                // Consume from buffer
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy(
                        (*c).buf.as_ptr().add(consumed),
                        (*c).buf.as_mut_ptr(),
                        leftover,
                    );
                    (*c).buf_len = leftover;
                    (*c).request_start = perf::stage_start();
                    continue; // Try to process next pipelined request
                }
                (*c).buf_len = 0;
                epoll_arm(fd, CLIENT_EVENTS);
                return true;
            } else if sent < 0 {
                let err = *libc::__errno_location();
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    perf::write_eagain();
                    (*c).send_ptr = resp.as_ptr();
                    (*c).send_len = resp.len();
                    (*c).send_off = 0;
                    store_pending_write(c, total_start, write_start, success);
                    let leftover = (*c).buf_len - consumed;
                    if leftover > 0 {
                        std::ptr::copy(
                            (*c).buf.as_ptr().add(consumed),
                            (*c).buf.as_mut_ptr(),
                            leftover,
                        );
                    }
                    (*c).buf_len = leftover;
                    epoll_arm(fd, CLIENT_WRITE_EVENTS);
                    return true;
                }
                drop_conn(fd);
                finish_request(c, total_start, false);
                return true;
            } else {
                // Partial send
                perf::partial_write();
                if sent > 0 {
                    perf::add_bytes_sent(sent as usize);
                }
                (*c).send_ptr = resp.as_ptr();
                (*c).send_len = resp.len();
                (*c).send_off = sent as usize;
                store_pending_write(c, total_start, write_start, success);
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy(
                        (*c).buf.as_ptr().add(consumed),
                        (*c).buf.as_mut_ptr(),
                        leftover,
                    );
                }
                (*c).buf_len = leftover;
                epoll_arm(fd, CLIENT_WRITE_EVENTS);
                return true;
            }
        }

        // GET /ready
        perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
        if buf.len() >= 10 && &buf[..3] == b"GET" {
            perf::record_stage(perf::STAGE_VALIDATION, validation_start);
            send_response_inline(fd, response::RESP_READY);
            finish_request(c, current_request_start(c, processing_start), true);
            epoll_arm(fd, (libc::EPOLLIN | libc::EPOLLRDHUP) as u32);
            return true;
        }

        perf::record_stage(perf::STAGE_VALIDATION, validation_start);
        send_response_inline(fd, response::RESP_NOT_FOUND);
        drop_conn(fd);
        finish_request(c, current_request_start(c, processing_start), false);
        return true;
    }
}

#[inline]
unsafe fn current_request_start(
    c: *mut Conn,
    fallback: Option<std::time::Instant>,
) -> Option<std::time::Instant> {
    (*c).request_start.or(fallback)
}

#[inline]
unsafe fn finish_request(c: *mut Conn, total_start: Option<std::time::Instant>, success: bool) {
    (*c).request_start = None;
    perf::record_stage(perf::STAGE_SERVER_PROCESSING, total_start);
    perf::record_request(total_start, success);
}

#[inline]
unsafe fn store_pending_write(
    c: *mut Conn,
    total_start: Option<std::time::Instant>,
    write_start: Option<std::time::Instant>,
    success: bool,
) {
    (*c).request_start = None;
    (*c).pending_request_start = total_start;
    (*c).pending_write_start = write_start;
    (*c).pending_success = success;
}

unsafe fn handle_client_read(fd: RawFd) {
    let idx = fd as usize;
    if idx >= MAX_FDS || CONNS[idx].is_null() {
        drop_conn(fd);
        return;
    }
    let c = CONNS[idx];

    // ET mode: drain the socket fully
    let recv_stage_start = perf::stage_start();
    loop {
        let room = BUF_CAP - (*c).buf_len;
        if room == 0 {
            break;
        }
        perf::recv_call();
        let n = libc::recv(
            fd,
            (*c).buf.as_mut_ptr().add((*c).buf_len) as *mut _,
            room,
            0,
        );
        if n > 0 {
            (*c).buf_len += n as usize;
            perf::add_bytes_received(n as usize);
        } else if n == 0 {
            perf::record_stage(perf::STAGE_SOCKET_RECV, recv_stage_start);
            drop_conn(fd);
            return;
        } else {
            let err = *libc::__errno_location();
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                break;
            }
            perf::record_stage(perf::STAGE_SOCKET_RECV, recv_stage_start);
            drop_conn(fd);
            return;
        }
    }
    perf::record_stage(perf::STAGE_SOCKET_RECV, recv_stage_start);

    // Process pipelined requests
    loop {
        let processing_start = perf::stage_start();
        let validation_start = perf::stage_start();
        let http_start = perf::stage_start();
        let buf = &(&(*c).buf)[..(*c).buf_len];
        let header_end = match find_double_crlf(buf) {
            Some(p) => p,
            None => {
                perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
                return;
            }
        };

        if buf.len() >= 5 && &buf[..5] == b"POST " {
            let cl = match content_length_fast(&buf[..header_end]) {
                Some(cl) => cl,
                None => {
                    perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
                    perf::record_stage(perf::STAGE_VALIDATION, validation_start);
                    send_and_close(fd, response::RESP_DENIED_S10);
                    finish_request(c, current_request_start(c, processing_start), false);
                    return;
                }
            };
            perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
            if buf.len() < header_end + cl {
                return; // Need more body
            }
            perf::record_stage(perf::STAGE_VALIDATION, validation_start);
            let body = &buf[header_end..header_end + cl];
            let (resp, success) = fraud_response(body);
            let consumed = header_end + cl;
            let total_start = current_request_start(c, processing_start);

            let write_start = perf::stage_start();
            perf::send_call();
            let sent = socket_write(fd, resp.as_ptr(), resp.len());
            perf::record_stage(perf::STAGE_WRITE_RESPONSE, write_start);
            if sent as usize == resp.len() {
                perf::add_bytes_sent(sent as usize);
                perf::record_stage(perf::STAGE_WRITE_COMPLETE, write_start);
                finish_request(c, total_start, success);
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy(
                        (*c).buf.as_ptr().add(consumed),
                        (*c).buf.as_mut_ptr(),
                        leftover,
                    );
                    (*c).buf_len = leftover;
                    (*c).request_start = perf::stage_start();
                    continue; // Pipeline
                }
                (*c).buf_len = 0;
                return;
            } else if sent < 0 {
                let err = *libc::__errno_location();
                if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                    perf::write_eagain();
                    (*c).send_ptr = resp.as_ptr();
                    (*c).send_len = resp.len();
                    (*c).send_off = 0;
                    store_pending_write(c, total_start, write_start, success);
                    let leftover = (*c).buf_len - consumed;
                    if leftover > 0 {
                        std::ptr::copy(
                            (*c).buf.as_ptr().add(consumed),
                            (*c).buf.as_mut_ptr(),
                            leftover,
                        );
                    }
                    (*c).buf_len = leftover;
                    epoll_arm(fd, CLIENT_WRITE_EVENTS);
                    return;
                }
                drop_conn(fd);
                finish_request(c, total_start, false);
                return;
            } else {
                // Partial send
                perf::partial_write();
                if sent > 0 {
                    perf::add_bytes_sent(sent as usize);
                }
                (*c).send_ptr = resp.as_ptr();
                (*c).send_len = resp.len();
                (*c).send_off = sent as usize;
                store_pending_write(c, total_start, write_start, success);
                let leftover = (*c).buf_len - consumed;
                if leftover > 0 {
                    std::ptr::copy(
                        (*c).buf.as_ptr().add(consumed),
                        (*c).buf.as_mut_ptr(),
                        leftover,
                    );
                }
                (*c).buf_len = leftover;
                epoll_arm(fd, CLIENT_WRITE_EVENTS);
                return;
            }
        } else if buf.len() >= 3 && &buf[..3] == b"GET" {
            let resp = response::RESP_READY;
            perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
            perf::record_stage(perf::STAGE_VALIDATION, validation_start);
            let write_start = perf::stage_start();
            perf::send_call();
            let sent = socket_write(fd, resp.as_ptr(), resp.len());
            perf::record_stage(perf::STAGE_WRITE_RESPONSE, write_start);
            if sent > 0 {
                perf::add_bytes_sent(sent as usize);
            }
            perf::record_stage(perf::STAGE_WRITE_COMPLETE, write_start);
            finish_request(c, current_request_start(c, processing_start), true);
            let leftover = (*c).buf_len - header_end;
            if leftover > 0 {
                std::ptr::copy(
                    (*c).buf.as_ptr().add(header_end),
                    (*c).buf.as_mut_ptr(),
                    leftover,
                );
                (*c).buf_len = leftover;
                continue;
            }
            (*c).buf_len = 0;
            return;
        } else {
            perf::record_stage(perf::STAGE_HTTP_PARSE, http_start);
            perf::record_stage(perf::STAGE_VALIDATION, validation_start);
            send_and_close(fd, response::RESP_NOT_FOUND);
            finish_request(c, current_request_start(c, processing_start), false);
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
        epoll_arm(fd, CLIENT_EVENTS);
        return;
    }

    // ET mode: drain send buffer fully
    loop {
        let remaining = (*c).send_len - (*c).send_off;
        if remaining == 0 {
            break;
        }
        let write_start = perf::stage_start();
        perf::send_call();
        let n = socket_write(fd, (*c).send_ptr.add((*c).send_off), remaining);
        perf::record_stage(perf::STAGE_WRITE_RESPONSE, write_start);
        if n > 0 {
            (*c).send_off += n as usize;
            perf::add_bytes_sent(n as usize);
        } else {
            let err = *libc::__errno_location();
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                perf::write_eagain();
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
    perf::record_stage(perf::STAGE_WRITE_COMPLETE, (*c).pending_write_start.take());
    finish_request(c, (*c).pending_request_start.take(), (*c).pending_success);
    (*c).pending_success = false;
    epoll_arm(fd, CLIENT_EVENTS);
}

#[inline]
unsafe fn send_response_inline(fd: RawFd, resp: &[u8]) {
    let write_start = perf::stage_start();
    perf::send_call();
    let sent = socket_write(fd, resp.as_ptr(), resp.len());
    perf::record_stage(perf::STAGE_WRITE_RESPONSE, write_start);
    if sent > 0 {
        perf::add_bytes_sent(sent as usize);
    }
    perf::record_stage(perf::STAGE_WRITE_COMPLETE, write_start);
}

#[inline]
unsafe fn send_and_close(fd: RawFd, resp: &[u8]) {
    send_response_inline(fd, resp);
    drop_conn(fd);
}

#[inline]
fn fraud_response(body: &[u8]) -> (&'static [u8], bool) {
    match extract(body) {
        Some(mut p) => {
            let cache_start = perf::stage_start();
            let fast_count = try_fast_fraud_count(&p);
            perf::record_stage(perf::STAGE_CACHE_LOOKUP, cache_start);
            if let Some(count) = fast_count {
                perf::cache_hit();
                let serialize_start = perf::stage_start();
                let resp = response::for_count(count);
                perf::record_stage(perf::STAGE_SERIALIZE, serialize_start);
                return (resp, true);
            }
            perf::cache_miss();
            let decision_start = perf::stage_start();
            complete_cache(&mut p);
            let count = tier_gray_count(&p);
            perf::record_stage(perf::STAGE_DECISION_TREE, decision_start);
            let serialize_start = perf::stage_start();
            let resp = response::for_count(count);
            perf::record_stage(perf::STAGE_SERIALIZE, serialize_start);
            (resp, true)
        }
        None => (response::RESP_DENIED_S10, false),
    }
}

fn create_unix_listener(path: &Path) -> anyhow::Result<RawFd> {
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
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
