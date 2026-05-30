use crate::index::Index;
use crate::ingest::extract_filled;
use crate::search::tier_fraud_count;
use crate::http::response;
use monoio::io::{AsyncReadRent, AsyncWriteRentExt};
use std::sync::Arc;

pub async fn handle_connection<S>(index: Arc<Index>, mut stream: S)
where
    S: AsyncReadRent + AsyncWriteRentExt,
{
    let mut req_buf: Vec<u8> = Vec::with_capacity(2048);
    let mut read_buf = vec![0u8; 8192];
    loop {
        let (res, tmp) = stream.read(read_buf).await;
        let n = match res {
            Ok(0) => return,
            Ok(n) => n,
            Err(_) => return,
        };
        req_buf.extend_from_slice(&tmp[..n]);
        read_buf = tmp;

        // Try to handle as many pipelined requests as we have full bytes for.
        loop {
            let outcome = try_handle_one(&index, &mut stream, &req_buf).await;
            match outcome {
                HandleResult::NeedMore => break,
                HandleResult::Consumed(c) => {
                    req_buf.drain(..c);
                }
                HandleResult::Drop => {
                    send_static(&mut stream, response::RESP_DENIED_S10).await;
                    return;
                }
            }
            if req_buf.is_empty() {
                break;
            }
        }
    }
}

enum HandleResult {
    NeedMore,
    Consumed(usize),
    Drop,
}

async fn try_handle_one<S>(index: &Arc<Index>, stream: &mut S, buf: &[u8]) -> HandleResult
where
    S: AsyncWriteRentExt,
{
    // Need full headers
    let header_end = match find_double_crlf(buf) {
        Some(p) => p,
        None => return HandleResult::NeedMore,
    };

    if buf.len() >= 21 && &buf[..21] == b"POST /fraud-score HTTP" {
        let cl = match content_length_fast(&buf[..header_end]) {
            Some(c) => c,
            None => {
                send_static(stream, response::RESP_DENIED_S10).await;
                return HandleResult::Consumed(header_end);
            }
        };
        if buf.len() < header_end + cl {
            return HandleResult::NeedMore;
        }
        let body = &buf[header_end..header_end + cl];
        send_static(stream, fraud_response(index, body)).await;
        return HandleResult::Consumed(header_end + cl);
    }

    let request_line_end = match memchr_crlf(buf) {
        Some(p) => p,
        None => return HandleResult::NeedMore,
    };
    let request_line = &buf[..request_line_end];

    let (method, path) = match parse_request_line(request_line) {
        Some((m, p)) => (m, p),
        None => {
            send_static(stream, response::RESP_DENIED_S10).await;
            return HandleResult::Consumed(header_end);
        }
    };

    if method == b"GET" && path == b"/ready" {
        send_static(stream, response::RESP_READY).await;
        return HandleResult::Consumed(header_end);
    }

    if method == b"POST" && path == b"/fraud-score" {
        let cl = match content_length(&buf[..header_end]) {
            Some(c) => c,
            None => {
                send_static(stream, response::RESP_DENIED_S10).await;
                return HandleResult::Consumed(header_end);
            }
        };
        if buf.len() < header_end + cl {
            return HandleResult::NeedMore;
        }
        let body = &buf[header_end..header_end + cl];
        send_static(stream, fraud_response(index, body)).await;
        return HandleResult::Consumed(header_end + cl);
    }

    send_static(stream, response::RESP_NOT_FOUND).await;
    HandleResult::Consumed(header_end)
}

#[inline]
fn fraud_response(_index: &Index, body: &[u8]) -> &'static [u8] {
    match extract_filled(body) {
        Some(p) => response::for_count(tier_fraud_count(&p)),
        None => response::RESP_DENIED_S10,
    }
}

async fn send_static<S: AsyncWriteRentExt>(stream: &mut S, payload: &'static [u8]) {
    // monoio's IoBuf is impl'd for &'static [u8], so we can hand the static slice directly
    // — zero allocation per response.
    let _ = stream.write_all(payload).await;
}

fn parse_request_line(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let sp1 = line.iter().position(|&c| c == b' ')?;
    let rest = &line[sp1 + 1..];
    let sp2 = rest.iter().position(|&c| c == b' ')?;
    Some((&line[..sp1], &rest[..sp2]))
}

#[inline]
fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    const MARK: u32 = 0x0a0d_0a0d;
    let len = buf.len();
    if len < 4 {
        return None;
    }
    let mut i = 0usize;
    while i + 4 <= len {
        let w = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        if w == MARK {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}

#[inline]
fn content_length_fast(headers: &[u8]) -> Option<usize> {
    const TAG: &[u8] = b"\r\nContent-Length: ";
    let mut i = 0usize;
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
    content_length(headers)
}

fn memchr_crlf(buf: &[u8]) -> Option<usize> {
    let len = buf.len();
    if len < 2 {
        return None;
    }
    let mut i = 0usize;
    while i + 1 < len {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn content_length(headers: &[u8]) -> Option<usize> {
    const PATTERNS: &[&[u8]] = &[b"\r\nContent-Length: ", b"\r\ncontent-length: "];
    for pattern in PATTERNS {
        if let Some(pos) = headers.windows(pattern.len()).position(|w| w == *pattern) {
            let start = pos + pattern.len();
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
    }
    None
}
