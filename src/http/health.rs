//! Healthcheck TCP mínimo (sem monoio) — cabe no hot path da submissão.

use std::io::{Read, Write};

use crate::http::response;

pub fn health_tcp_loop(port: u16) {
    let listener = std::net::TcpListener::bind(format!("0.0.0.0:{port}")).expect("health bind");
    eprintln!("healthcheck TCP on :{port}");
    for stream in listener.incoming().flatten() {
        let _ = stream.set_nodelay(true);
        let mut stream = stream;
        let mut buf = [0u8; 512];
        if let Ok(n) = stream.read(&mut buf) {
            if n > 0 && buf[..n].windows(6).any(|w| w == b"/ready") {
                let _ = stream.write_all(response::RESP_READY);
            }
        }
    }
}
