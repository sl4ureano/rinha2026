//! HTTP bloqueante por conexão (pthread-style).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::io::{FromRawFd, RawFd};

use crate::ingest::extract;
use crate::http::response;
use crate::search::tier_fraud_count;

const REQ_CAP: usize = 65536;

pub fn serve_connection(fd: RawFd) {
    let mut stream = unsafe { TcpStream::from_raw_fd(fd) };
    let _ = stream.set_nodelay(true);

    let mut req_buf = Vec::with_capacity(4096);
    let mut read_buf = [0u8; 8192];

    loop {
        let n = match stream.read(&mut read_buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if req_buf.len() + n > REQ_CAP {
            return;
        }
        req_buf.extend_from_slice(&read_buf[..n]);

        loop {
            match try_handle_one(&mut stream, &req_buf) {
                HandleOutcome::NeedMore => break,
                HandleOutcome::Consumed(c) => {
                    req_buf.drain(..c);
                }
                HandleOutcome::Drop => {
                    let _ = write_resp(&mut stream, response::RESP_DENIED_S10);
                    return;
                }
            }
            if req_buf.is_empty() {
                break;
            }
        }
    }
}

enum HandleOutcome {
    NeedMore,
    Consumed(usize),
    Drop,
}

fn try_handle_one(stream: &mut TcpStream, buf: &[u8]) -> HandleOutcome {
    let header_end = match find_double_crlf(buf) {
        Some(p) => p,
        None => return HandleOutcome::NeedMore,
    };

    if buf.len() >= 21 && &buf[..21] == b"POST /fraud-score HTTP" {
        let cl = match content_length_fast(&buf[..header_end]) {
            Some(c) => c,
            None => {
                let _ = write_resp(stream, response::RESP_DENIED_S10);
                return HandleOutcome::Consumed(header_end);
            }
        };
        if buf.len() < header_end + cl {
            return HandleOutcome::NeedMore;
        }
        let body = &buf[header_end..header_end + cl];
        let fc = fraud_count_body(body);
        let _ = write_resp(stream, response::for_count(fc));
        return HandleOutcome::Consumed(header_end + cl);
    }

    let request_line_end = match memchr_crlf(buf) {
        Some(p) => p,
        None => return HandleOutcome::NeedMore,
    };
    let (method, path) = match parse_request_line(&buf[..request_line_end]) {
        Some(x) => x,
        None => {
            let _ = write_resp(stream, response::RESP_DENIED_S10);
            return HandleOutcome::Consumed(header_end);
        }
    };

    if method == b"GET" && path == b"/ready" {
        let _ = write_resp(stream, response::RESP_READY);
        return HandleOutcome::Consumed(header_end);
    }

    if method == b"POST" && path == b"/fraud-score" {
        let cl = match content_length_fast(&buf[..header_end]) {
            Some(c) => c,
            None => {
                let _ = write_resp(stream, response::RESP_DENIED_S10);
                return HandleOutcome::Consumed(header_end);
            }
        };
        if buf.len() < header_end + cl {
            return HandleOutcome::NeedMore;
        }
        let body = &buf[header_end..header_end + cl];
        let fc = fraud_count_body(body);
        let _ = write_resp(stream, response::for_count(fc));
        return HandleOutcome::Consumed(header_end + cl);
    }

    let _ = write_resp(stream, response::RESP_NOT_FOUND);
    HandleOutcome::Consumed(header_end)
}

#[inline]
fn fraud_count_body(body: &[u8]) -> u8 {
    match extract(body) {
        Some(p) => tier_fraud_count(&p),
        None => 5,
    }
}

fn write_resp(stream: &mut TcpStream, payload: &[u8]) -> std::io::Result<()> {
    stream.write_all(payload)
}

fn parse_request_line(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let sp1 = line.iter().position(|&c| c == b' ')?;
    let rest = &line[sp1 + 1..];
    let sp2 = rest.iter().position(|&c| c == b' ')?;
    Some((&line[..sp1], &rest[..sp2]))
}

#[inline]
fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    for i in 0..=buf.len() - 4 {
        let w = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        if w == 0x0a0d_0a0d {
            return Some(i + 4);
        }
    }
    None
}

#[inline]
fn content_length_fast(headers: &[u8]) -> Option<usize> {
    const TAG: &[u8] = b"\r\nContent-Length: ";
    for i in 0..=headers.len().saturating_sub(TAG.len()) {
        if &headers[i..i + TAG.len()] != TAG {
            continue;
        }
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
    None
}

fn memchr_crlf(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len().saturating_sub(1) {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}
