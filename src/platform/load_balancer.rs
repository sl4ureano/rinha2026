//! Single-thread blocking-accept load balancer with multiple upstreams.

use std::os::fd::RawFd;

use crate::config::LbConfig;
use crate::platform::scm::{connect_unix_retry, connect_unix_once, send_fd, set_tcp_nodelay};

const BACKLOG: libc::c_int = 65535;

struct Upstream {
    path: String,
    fd: RawFd,
}

impl Upstream {
    fn reconnect(&mut self) -> bool {
        if self.fd >= 0 {
            unsafe { libc::close(self.fd) };
            self.fd = -1;
        }
        for _ in 0..20 {
            match connect_unix_once(&self.path) {
                Ok(fd) => {
                    self.fd = fd;
                    return true;
                }
                Err(_) => {
                    let ts = libc::timespec {
                        tv_sec: 0,
                        tv_nsec: 2_000_000,
                    };
                    unsafe { libc::nanosleep(&ts, std::ptr::null_mut()) };
                }
            }
        }
        false
    }
}

fn handoff(upstream: &mut Upstream, client_fd: RawFd) -> bool {
    if upstream.fd < 0 && !upstream.reconnect() {
        return false;
    }
    if send_fd(upstream.fd, client_fd) {
        return true;
    }
    if !upstream.reconnect() {
        return false;
    }
    send_fd(upstream.fd, client_fd)
}

pub fn run(cfg: LbConfig) {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };

    assert!(!cfg.upstreams.is_empty(), "lb: no upstreams configured");

    let mut upstreams: Vec<Upstream> = cfg
        .upstreams
        .iter()
        .map(|path| {
            let fd = connect_unix_retry(path);
            eprintln!("lb: connected upstream -> {path}");
            Upstream {
                path: path.clone(),
                fd,
            }
        })
        .collect();

    let upstream_count = upstreams.len();
    eprintln!("lb: {} upstreams configured", upstream_count);

    let listen_fd = tcp_listen(cfg.port).expect("listen");
    eprintln!(
        "lb: listening on port {} (backlog={}, upstreams={})",
        cfg.port, BACKLOG, upstream_count
    );

    let mut rr_next: u32 = 0;

    loop {
        let client = unsafe {
            libc::accept4(
                listen_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            )
        };
        if client < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            continue;
        }
        // Zero socket options here — API sets them after receiving the fd.
        // This makes the LB path: accept4 → sendmsg → close (3 syscalls).

        let first = (rr_next % upstream_count as u32) as usize;
        rr_next = rr_next.wrapping_add(1);

        if !handoff(&mut upstreams[first], client) {
            for offset in 1..upstream_count {
                if handoff(&mut upstreams[(first + offset) % upstream_count], client) {
                    break;
                }
            }
        }
        unsafe { libc::close(client) };
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
        libc::setsockopt(
            sock,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &one as *const _ as *const _,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        libc::setsockopt(
            sock,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &one as *const _ as *const _,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        // Increase SO_SNDBUF on the control sockets for smoother FD passing
        let sndbuf: libc::c_int = 262144;
        libc::setsockopt(
            sock,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &sndbuf as *const _ as *const _,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        // TCP_DEFER_ACCEPT: kernel holds conn until data arrives → faster accept
        let defer: libc::c_int = 1;
        libc::setsockopt(
            sock,
            libc::IPPROTO_TCP,
            libc::TCP_DEFER_ACCEPT,
            &defer as *const _ as *const _,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        // TCP_FASTOPEN: allow data in SYN for new connections
        let tfo: libc::c_int = 5;
        libc::setsockopt(
            sock,
            libc::IPPROTO_TCP,
            libc::TCP_FASTOPEN,
            &tfo as *const _ as *const _,
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
    if unsafe { libc::listen(sock, BACKLOG) } != 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(sock) };
        return Err(e);
    }
    Ok(sock)
}
