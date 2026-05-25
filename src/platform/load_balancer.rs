//! Single-thread epoll load balancer: accepts TCP and passes FDs to API workers.

use std::os::fd::RawFd;

use crate::config::LbConfig;
use crate::platform::scm::{connect_unix_retry, send_fd, set_nonblocking, set_tcp_nodelay, write_502};

pub fn run(cfg: LbConfig) {
    let ctrl1 = connect_unix_retry(&cfg.api1_socket);
    let ctrl2 = connect_unix_retry(&cfg.api2_socket);
    eprintln!("lb epoll :{} upstreams ready", cfg.port);

    let listen_fd = tcp_listen(cfg.port).expect("listen");
    set_nonblocking(listen_fd);
    let epfd = epoll_create().expect("epoll_create");
    epoll_add(epfd, listen_fd).expect("epoll_add");

    let mut events = [libc::epoll_event { events: 0, u64: 0 }; 64];
    let mut next_upstream: usize = 0;
    let ctrl = [ctrl1, ctrl2];

    loop {
        let n = unsafe {
            libc::epoll_wait(epfd, events.as_mut_ptr(), events.len() as i32, -1)
        };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        for i in 0..n as usize {
            if (events[i].events & libc::EPOLLIN as u32) == 0 {
                continue;
            }
            loop {
                let client = unsafe {
                    libc::accept4(
                        listen_fd,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        libc::SOCK_CLOEXEC,
                    )
                };
                if client < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.raw_os_error() == Some(libc::EAGAIN)
                    {
                        break;
                    }
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                set_tcp_nodelay(client);
                let primary = next_upstream;
                next_upstream ^= 1;
                if !send_fd(ctrl[primary], client) && !send_fd(ctrl[primary ^ 1], client) {
                    write_502(client);
                }
                unsafe { libc::close(client) };
            }
        }
    }
}

fn epoll_create() -> std::io::Result<RawFd> {
    let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn epoll_add(epfd: RawFd, fd: RawFd) -> std::io::Result<()> {
    let mut ev = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: fd as u64,
    };
    let r = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, fd, &mut ev) };
    if r != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn tcp_listen(port: u16) -> std::io::Result<RawFd> {
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if sock < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let one: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            sock,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &one as *const _ as *const _,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as libc::sa_family_t;
    addr.sin_addr.s_addr = u32::to_be(libc::INADDR_ANY);
    addr.sin_port = port.to_be();
    if unsafe {
        libc::bind(
            sock,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    } != 0
    {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(sock) };
        return Err(e);
    }
    if unsafe { libc::listen(sock, 16384) } != 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(sock) };
        return Err(e);
    }
    Ok(sock)
}
