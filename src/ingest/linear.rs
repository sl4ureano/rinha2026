//! Extrator linear para JSON compacto do k6 (sem scanner genérico).
//! Se o layout não bater, `extract` em `json.rs` cai no parser completo.

use super::json::RawPayload;
use super::numbers::{parse_f32_bytes, parse_u32_bytes};

/// Layout típico: `{"id":"tx-…","transaction":{…},…}` sem espaços.
#[inline]
fn looks_compact_rinha(body: &[u8]) -> bool {
    body.len() >= 80
        && body.first() == Some(&b'{')
        && memchr::memmem::find(body, b"\"transaction\":{\"amount\":").is_some()
        && !memchr::memchr(b'\n', &body[..body.len().min(256)]).is_some()
}

#[inline]
fn needle(body: &[u8], tag: &[u8], from: usize) -> Option<usize> {
    let i = memchr::memmem::find(&body[from..], tag)?;
    Some(from + i + tag.len())
}

#[inline]
fn parse_f32(body: &[u8], i: usize) -> Option<(f32, usize)> {
    let start = i;
    let mut j = i;
    while j < body.len() {
        let b = body[j];
        if matches!(b, b'-' | b'+' | b'.' | b'0'..=b'9') {
            j += 1;
        } else {
            break;
        }
    }
    let v = parse_f32_bytes(&body[start..j])?;
    Some((v, j))
}

#[inline]
fn parse_u32(body: &[u8], i: usize) -> Option<(u32, usize)> {
    let start = i;
    let mut j = i;
    while j < body.len() && body[j].is_ascii_digit() {
        j += 1;
    }
    let v = parse_u32_bytes(&body[start..j])?;
    Some((v, j))
}

#[inline]
fn parse_bool(body: &[u8], i: usize) -> Option<(bool, usize)> {
    if body[i..].starts_with(b"true") {
        Some((true, i + 4))
    } else if body[i..].starts_with(b"false") {
        Some((false, i + 5))
    } else {
        None
    }
}

#[inline]
fn parse_quoted(body: &[u8], i: usize) -> Option<(&[u8], usize)> {
    if body.get(i) != Some(&b'"') {
        return None;
    }
    let start = i + 1;
    let mut j = start;
    while j < body.len() {
        match body[j] {
            b'"' => return Some((&body[start..j], j + 1)),
            b'\\' if j + 1 < body.len() => j += 2,
            _ => j += 1,
        }
    }
    None
}

/// Slice do conteúdo interno de `known_merchants:[…]` (sem colchetes).
fn parse_known_merchants_inner(body: &[u8], i: usize) -> Option<(&[u8], usize)> {
    let mut j = i;
    while j < body.len() && body[j].is_ascii_whitespace() {
        j += 1;
    }
    if body.get(j) != Some(&b'[') {
        return None;
    }
    let inner_start = j + 1;
    let mut k = inner_start;
    let mut depth = 1usize;
    let mut in_str = false;
    while k < body.len() {
        let c = body[k];
        if in_str {
            if c == b'\\' {
                k += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            k += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                k += 1;
            }
            b'[' => {
                depth += 1;
                k += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&body[inner_start..k], k + 1));
                }
                k += 1;
            }
            _ => k += 1,
        }
    }
    None
}

/// Extrai campos na ordem do payload da prova; `None` → usar scanner.
pub fn extract_linear(body: &[u8]) -> Option<RawPayload<'_>> {
    if !looks_compact_rinha(body) {
        return None;
    }

    let mut p = RawPayload::default();
    let mut i = needle(body, b"\"transaction\":{\"amount\":", 0)?;
    (p.amount, i) = parse_f32(body, i)?;

    i = needle(body, b"\"installments\":", i)?;
    (p.installments, i) = parse_u32(body, i)?;

    i = needle(body, b"\"requested_at\":\"", i)?;
    (p.requested_at, i) = parse_quoted(body, i)?;

    i = needle(body, b"\"customer\":{\"avg_amount\":", i)?;
    (p.customer_avg_amount, i) = parse_f32(body, i)?;

    i = needle(body, b"\"tx_count_24h\":", i)?;
    (p.tx_count_24h, i) = parse_u32(body, i)?;

    i = needle(body, b"\"known_merchants\":", i)?;
    (p.known_merchants, i) = parse_known_merchants_inner(body, i)?;

    i = needle(body, b"\"merchant\":{\"id\":\"", i)?;
    (p.merchant_id, i) = parse_quoted(body, i)?;

    i = needle(body, b"\"mcc\":\"", i)?;
    (p.merchant_mcc, i) = parse_quoted(body, i)?;

    i = needle(body, b"\"avg_amount\":", i)?;
    (p.merchant_avg_amount, i) = parse_f32(body, i)?;

    i = needle(body, b"\"terminal\":{\"is_online\":", i)?;
    (p.is_online, i) = parse_bool(body, i)?;
    i = needle(body, b"\"card_present\":", i)?;
    (p.card_present, i) = parse_bool(body, i)?;
    i = needle(body, b"\"km_from_home\":", i)?;
    (p.km_from_home, i) = parse_f32(body, i)?;

    let last_off = memchr::memmem::find(body, b"\"last_transaction\":")?;
    let tail = &body[last_off + b"\"last_transaction\":".len()..];
    if tail.starts_with(b"null") {
        // ok
    } else if let Some(ts_off) = memchr::memmem::find(tail, b"\"timestamp\":\"") {
        let ts_start = last_off + b"\"last_transaction\":".len() + ts_off + b"\"timestamp\":\"".len();
        let (ts, mut j) = parse_quoted(body, ts_start)?;
        p.last_timestamp = Some(ts);
        if let Some(km_off) = memchr::memmem::find(&body[j..], b"\"km_from_current\":") {
            j += km_off + b"\"km_from_current\":".len();
            p.last_km = Some(parse_f32(body, j)?.0);
        }
    } else {
        return None;
    }

    Some(p)
}
