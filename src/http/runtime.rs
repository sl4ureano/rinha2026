//! monoio TCP runtime (direct bind mode, without FD pass-through).

use std::sync::Arc;

use monoio::net::TcpListener;
use socket2::{Domain, Protocol, Socket, Type};

use crate::http;
use crate::index::Index;

type Driver = monoio::LegacyDriver;

pub fn run_tcp(index: Arc<Index>, bind_addr: std::net::SocketAddr) {
    let domain = if bind_addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).unwrap();
    sock.set_reuse_address(true).unwrap();
    sock.set_nodelay(true).unwrap();
    sock.set_nonblocking(true).unwrap();
    sock.bind(&bind_addr.into()).unwrap();
    sock.listen(1024).unwrap();
    let std_listener: std::net::TcpListener = sock.into();

    let mut rt = monoio::RuntimeBuilder::<Driver>::new()
        .enable_timer()
        .build()
        .expect("monoio runtime");
    rt.block_on(async move {
        let listener = TcpListener::from_std(std_listener).expect("TcpListener::from_std");
        eprintln!("listening tcp {bind_addr}");
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let index = index.clone();
                    monoio::spawn(async move {
                        http::handle_connection(index, stream).await;
                    });
                }
                Err(e) => eprintln!("accept error: {e}"),
            }
        }
    });
}

