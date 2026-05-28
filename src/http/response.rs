pub const RESP_READY: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";

// Minimal headers: HTTP/1.1 defaults to keep-alive; k6 parses body via JSON.parse, no Content-Type needed.
pub const RESP_APPROVED_S0: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 33\r\n\r\n{\"approved\":true,\"fraud_score\":0}";
pub const RESP_APPROVED_S2: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 35\r\n\r\n{\"approved\":true,\"fraud_score\":0.2}";
pub const RESP_APPROVED_S4: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 35\r\n\r\n{\"approved\":true,\"fraud_score\":0.4}";
pub const RESP_DENIED_S6: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 36\r\n\r\n{\"approved\":false,\"fraud_score\":0.6}";
pub const RESP_DENIED_S8: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 36\r\n\r\n{\"approved\":false,\"fraud_score\":0.8}";
pub const RESP_DENIED_S10: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 34\r\n\r\n{\"approved\":false,\"fraud_score\":1}";

#[inline]
pub fn for_count(count: u8) -> &'static [u8] {
    match count {
        0 => RESP_APPROVED_S0,
        1 => RESP_APPROVED_S2,
        2 => RESP_APPROVED_S4,
        3 => RESP_DENIED_S6,
        4 => RESP_DENIED_S8,
        _ => RESP_DENIED_S10,
    }
}

pub const RESP_NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_content_length(resp: &[u8]) {
        let sep = b"\r\n\r\n";
        let pos = resp.windows(4).position(|w| w == sep).unwrap();
        let body = &resp[pos + 4..];
        let header = std::str::from_utf8(&resp[..pos]).unwrap();
        let cl_line = header
            .lines()
            .find(|l| l.starts_with("Content-Length:"))
            .unwrap();
        let cl: usize = cl_line.split(':').nth(1).unwrap().trim().parse().unwrap();
        assert_eq!(cl, body.len(), "header CL {} != body {}", cl, body.len());
    }

    #[test]
    fn cl_s0() {
        assert_content_length(RESP_APPROVED_S0);
    }
    #[test]
    fn cl_s2() {
        assert_content_length(RESP_APPROVED_S2);
    }
    #[test]
    fn cl_s4() {
        assert_content_length(RESP_APPROVED_S4);
    }
    #[test]
    fn cl_s6() {
        assert_content_length(RESP_DENIED_S6);
    }
    #[test]
    fn cl_s8() {
        assert_content_length(RESP_DENIED_S8);
    }
    #[test]
    fn cl_s10() {
        assert_content_length(RESP_DENIED_S10);
    }
}
