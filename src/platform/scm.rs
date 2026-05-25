//! Unix domain sockets and `SCM_RIGHTS` file-descriptor passing.

use std::cell::RefCell;
use std::os::fd::RawFd;

thread_local! {
    static CMSG_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::new());
}

pub fn connect_unix_retry(path: &str) -> RawFd {
    loop {
        match connect_unix_once(path) {
            Ok(fd) => return fd,
            Err(e) => {
                let raw = e.raw_os_error().unwrap_or(0);
                if raw == libc::ENOENT || raw == libc::ECONNREFUSED || raw == libc::EAGAIN {
                    unsafe { libc::usleep(100_000) };
                    continue;
                }
                panic!("connect {path}: {e}");
            }
        }
    }
}

pub fn connect_unix_once(path: &str) -> std::io::Result<RawFd> {
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_bytes();
    if bytes.len() >= addr.sun_path.len() {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::other("unix path too long"));
    }
    for (i, &b) in bytes.iter().enumerate() {
        addr.sun_path[i] = b as libc::c_char;
    }
    if unsafe {
        libc::connect(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    } != 0
    {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    Ok(fd)
}

pub fn set_nonblocking(fd: RawFd) {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags >= 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }
}

pub fn set_tcp_nodelay(fd: RawFd) {
    let one: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &one as *const _ as *const _,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

/// Send `client_fd` to a control socket via `SCM_RIGHTS` (1-byte iov payload).
pub fn send_fd(ctrl_fd: RawFd, client_fd: RawFd) -> bool {
    let byte: u8 = 0;
    let mut iov = libc::iovec {
        iov_base: &byte as *const _ as *mut _,
        iov_len: 1,
    };
    let cmsg_space =
        unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as libc::c_uint) as usize };

    CMSG_BUF.with(|cell| {
        let mut cmsg_buf = cell.borrow_mut();
        if cmsg_buf.len() < cmsg_space {
            cmsg_buf.resize(cmsg_space, 0);
        }
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg_buf.as_mut_ptr() as *mut _;
        msg.msg_controllen = cmsg_buf.len();
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            if cmsg.is_null() {
                return false;
            }
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::copy_nonoverlapping(
                &client_fd as *const RawFd as *const u8,
                libc::CMSG_DATA(cmsg),
                std::mem::size_of::<RawFd>(),
            );
            msg.msg_controllen = (*cmsg).cmsg_len;
            loop {
                let n = libc::sendmsg(ctrl_fd, &msg, libc::MSG_NOSIGNAL);
                if n == 1 {
                    return true;
                }
                if n < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    if e.raw_os_error() == Some(libc::EPIPE) {
                        return false;
                    }
                }
                return false;
            }
        }
    })
}

pub fn write_502(fd: RawFd) {
    const RESP: &[u8] =
        b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let mut p = RESP.as_ptr();
    let mut n = RESP.len();
    while n > 0 {
        let w = unsafe { libc::write(fd, p as *const _, n) };
        if w <= 0 {
            return;
        }
        p = unsafe { p.add(w as usize) };
        n -= w as usize;
    }
}
