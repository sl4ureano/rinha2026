//! Precomputed fields for tier_score / fast_path (filled once in `extract`).

use super::RawPayload;

pub const TREE_FEATURE_COUNT: usize = 21;

const AMOUNT_VS_AVG_RATIO: f32 = 10.0;
const RATIO_FRAUD_THRESHOLD: f32 = 0.06951915;

#[derive(Clone, Copy, Debug)]
pub struct TierCache {
    pub mcc_u32: u32,
    pub merchant_known: bool,
    pub requested_valid: bool,
    pub req_hour: u8,
    pub req_weekday: u8,
    pub req_epoch: i64,
    pub last_present: bool,
    pub last_epoch_ok: bool,
    pub last_epoch: i64,
    /// Precomputed ratio path result (0 or 5).
    pub ratio_count: u8,
    /// Gray requests that never touch the decision tree.
    pub gray_ratio_only: bool,
    /// Tree features ready (set by `complete_cache`).
    pub tree_ready: bool,
    pub tree_features: [f32; TREE_FEATURE_COUNT],
}

impl Default for TierCache {
    fn default() -> Self {
        Self {
            mcc_u32: u32::MAX,
            merchant_known: false,
            requested_valid: false,
            req_hour: 0,
            req_weekday: 0,
            req_epoch: 0,
            last_present: false,
            last_epoch_ok: false,
            last_epoch: 0,
            ratio_count: 0,
            gray_ratio_only: false,
            tree_ready: false,
            tree_features: [0.0; TREE_FEATURE_COUNT],
        }
    }
}

/// Campos baratos: MCC, loja conhecida, ratio (não precisa de árvore).
pub fn fill_base(p: &RawPayload<'_>) -> TierCache {
    let safe_avg = p.customer_avg_amount.max(1.0);
    let norm = clamp01((p.amount / safe_avg) / AMOUNT_VS_AVG_RATIO);
    let ratio_count = if norm > RATIO_FRAUD_THRESHOLD { 5 } else { 0 };
    TierCache {
        mcc_u32: mcc4_u32(p.merchant_mcc),
        merchant_known: merchant_known(p),
        ratio_count,
        ..TierCache::default()
    }
}

/// ISO + flags de gray area — só depois que fast path óbvio falhar.
pub fn fill_datetime(p: &RawPayload<'_>, c: &mut TierCache) {
    if let Some(parsed) = parse_iso(p.requested_at) {
        c.requested_valid = true;
        c.req_hour = parsed.0;
        c.req_weekday = parsed.1;
        c.req_epoch = parsed.2;
    }

    if let Some(ts) = p.last_timestamp {
        c.last_present = true;
        if let Some((_, _, epoch)) = parse_iso(ts) {
            c.last_epoch_ok = true;
            c.last_epoch = epoch;
        }
    }

    c.gray_ratio_only =
        !c.requested_valid || (c.last_present && !c.last_epoch_ok);
}

/// Compat: base + datetime (ferramentas offline).
pub fn fill(p: &RawPayload<'_>) -> TierCache {
    let mut c = fill_base(p);
    fill_datetime(p, &mut c);
    c
}

#[inline]
fn clamp01(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}

#[inline]
pub fn mcc4_u32(mcc: &[u8]) -> u32 {
    if mcc.len() != 4 {
        return u32::MAX;
    }
    u32::from_be_bytes([mcc[0], mcc[1], mcc[2], mcc[3]])
}

#[inline]
fn merchant_known(p: &RawPayload<'_>) -> bool {
    contains_quoted(p.known_merchants, p.merchant_id)
}

#[inline]
fn contains_quoted(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() + 2 {
        return false;
    }
    if needle.len() + 2 <= 34 {
        let mut pat = [0u8; 36];
        pat[0] = b'"';
        pat[1..1 + needle.len()].copy_from_slice(needle);
        pat[1 + needle.len()] = b'"';
        let pat = &pat[..needle.len() + 2];
        if let Some(found) = memchr::memmem::find(haystack, pat) {
            return found + pat.len() <= haystack.len();
        }
        return false;
    }
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i + 1;
        if start + needle.len() + 1 <= haystack.len()
            && haystack[start..start + needle.len()] == *needle
            && haystack.get(start + needle.len()) == Some(&b'"')
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Returns `(hour, weekday_monday0, epoch_seconds)`.
fn parse_iso(ts: &[u8]) -> Option<(u8, u8, i64)> {
    if ts.len() < 19 {
        return None;
    }
    let year = digit4(ts[0], ts[1], ts[2], ts[3])? as i64;
    if ts[4] != b'-' || ts[7] != b'-' || ts[10] != b'T' || ts[13] != b':' {
        return None;
    }
    let month = digit2(ts[5], ts[6])? as i64;
    let day = digit2(ts[8], ts[9])? as i64;
    let hour = digit2(ts[11], ts[12])? as i64;
    let minute = digit2(ts[14], ts[15])? as i64;
    let second = digit2(ts[17], ts[18])? as i64;
    let days = days_from_civil(year, month, day);
    let weekday = ((days + 3).rem_euclid(7)) as u8;
    Some((
        hour as u8,
        weekday,
        days * 86_400 + hour * 3600 + minute * 60 + second,
    ))
}

#[inline]
fn digit2(a: u8, b: u8) -> Option<u32> {
    if a.is_ascii_digit() && b.is_ascii_digit() {
        Some((a - b'0') as u32 * 10 + (b - b'0') as u32)
    } else {
        None
    }
}

#[inline]
fn digit4(a: u8, b: u8, c: u8, d: u8) -> Option<u32> {
    Some(digit2(a, b)? * 100 + digit2(c, d)?)
}

#[inline]
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let mut year = y;
    let month = m;
    if month <= 2 {
        year -= 1;
    }
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_adj = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * month_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
