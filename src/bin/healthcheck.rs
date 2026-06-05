use std::io::{Read, Write};
use std::process;

fn main() {
    check_tcp("8080");
}

fn check_tcp(port: &str) {
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = match std::net::TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(_) => process::exit(1),
    };
    if stream.write_all(b"GET /ready HTTP/1.0\r\n\r\n").is_err() {
        process::exit(1);
    }
    let mut buf = [0u8; 256];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 && buf[..n].windows(2).any(|w| w == b"OK") => process::exit(0),
        _ => process::exit(1),
    }
}
