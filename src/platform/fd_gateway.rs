//! FD-passing receiver: LB envia TCP FDs via SCM_RIGHTS; uma thread bloqueante por conexão.

use anyhow::Result;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::sync::Arc;

use crate::index::Index;

pub fn run(_index: Arc<Index>, sock_path: &Path) -> Result<()> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(sock_path);

    let listener_fd = create_unix_listener(sock_path)?;
    eprintln!("listening on uds fd-passing {}", sock_path.display());

    accept_loop(listener_fd);
}

fn spawn_conn(fd: RawFd) {
    set_tcp_nodelay_fd(fd);
    std::thread::Builder::new()
        .name("http-conn".into())
        .stack_size(256 * 1024)
        .spawn(move || crate::http::serve_connection(fd))
        .ok();
}

fn create_unix_listener(path: &Path) -> Result<libc::c_int> {
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
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

fn accept_loop(listener_fd: libc::c_int) -> ! {
    loop {
        let control = unsafe {
            libc::accept4(
                listener_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        if control < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            eprintln!("fd-accept error: {err}");
            continue;
        }
        std::thread::Builder::new()
            .name("fd-recv".into())
            .spawn(move || recv_loop(control))
            .expect("spawn fd-recv");
    }
}

fn recv_loop(control_fd: libc::c_int) {
    loop {
        let Some(fd) = recv_fd(control_fd) else {
            break;
        };
        spawn_conn(fd);
        while let Some(fd) = recv_fd_nb(control_fd) {
            spawn_conn(fd);
        }
    }
    unsafe { libc::close(control_fd) };
}

fn set_tcp_nodelay_fd(fd: RawFd) {
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

fn recv_fd(control_fd: libc::c_int) -> Option<RawFd> {
    recv_fd_flags(control_fd, 0)
}

fn recv_fd_nb(control_fd: libc::c_int) -> Option<RawFd> {
    recv_fd_flags(control_fd, libc::MSG_DONTWAIT)
}

fn recv_fd_flags(control_fd: libc::c_int, flags: libc::c_int) -> Option<RawFd> {
    let mut one: u8 = 0;
    let mut iov = libc::iovec {
        iov_base: &mut one as *mut _ as *mut _,
        iov_len: 1,
    };
    let mut cmsg_buf = [0u8; 64];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut _;
    msg.msg_controllen = cmsg_buf.len();
    loop {
        let n = unsafe { libc::recvmsg(control_fd, &mut msg, flags) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            if flags & libc::MSG_DONTWAIT != 0
                && (e.kind() == std::io::ErrorKind::WouldBlock
                    || e.raw_os_error() == Some(libc::EAGAIN))
            {
                return None;
            }
            return None;
        }
        if n == 0 {
            return None;
        }
        unsafe {
            let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
            while !cmsg.is_null() {
                if (*cmsg).cmsg_level == libc::SOL_SOCKET
                    && (*cmsg).cmsg_type == libc::SCM_RIGHTS
                    && (*cmsg).cmsg_len
                        >= libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as u32) as _
                {
                    let mut fd: libc::c_int = -1;
                    std::ptr::copy_nonoverlapping(
                        libc::CMSG_DATA(cmsg) as *const u8,
                        &mut fd as *mut libc::c_int as *mut u8,
                        std::mem::size_of::<libc::c_int>(),
                    );
                    return Some(fd);
                }
                cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
            }
        }
    }
}
