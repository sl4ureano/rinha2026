//! Scorer em camadas: gasto seguro → gasto arriscado → árvore → ratio (sem k-NN em runtime).

use crate::ingest::RawPayload;
use crate::search::decision_tree::{self, FEATURE_COUNT};

const MAX_AMOUNT: f32 = 10_000.0;
const MAX_INSTALLMENTS: f32 = 12.0;
const AMOUNT_VS_AVG_RATIO: f32 = 10.0;
const MAX_MINUTES: f32 = 1440.0;
const MAX_KM: f32 = 1000.0;
const MAX_TX24H: f32 = 20.0;
const MAX_MERCHANT_AVG: f32 = 10_000.0;
const RATIO_FRAUD_THRESHOLD: f32 = 0.06951915;

/// Contagem 0–5 para respostas HTTP estáticas (0 = aprova, 5 = nega).
pub fn tier_fraud_count(p: &RawPayload<'_>) -> u8 {
    if obvious_legit(p) {
        return 0;
    }
    if obvious_fraud(p) {
        return 5;
    }
    if let Some(features) = build_tree_features(p) {
        return if decision_tree::predict(&features) { 5 } else { 0 };
    }
    ratio_fraud_count(p)
}

#[inline]
fn obvious_legit(p: &RawPayload<'_>) -> bool {
    if p.amount > 500.0 {
        return false;
    }
    let safe_avg = p.customer_avg_amount.max(1.0);
    if p.amount / safe_avg > 0.50001 {
        return false;
    }
    if p.installments > 3 {
        return false;
    }
    if p.tx_count_24h > 5 {
        return false;
    }
    if !merchant_known(p) {
        return false;
    }
    if p.km_from_home > 50.0 {
        return false;
    }
    is_safe_mcc(p.merchant_mcc)
}

#[inline]
fn obvious_fraud(p: &RawPayload<'_>) -> bool {
    p.amount >= 5000.0
        && p.installments >= 5
        && p.tx_count_24h >= 6
        && !merchant_known(p)
        && p.km_from_home >= 150.0
        && is_risky_mcc(p.merchant_mcc)
}

#[inline]
fn ratio_fraud_count(p: &RawPayload<'_>) -> u8 {
    let safe_avg = p.customer_avg_amount.max(1.0);
    let norm = clamp01((p.amount / safe_avg) / AMOUNT_VS_AVG_RATIO);
    if norm > RATIO_FRAUD_THRESHOLD {
        5
    } else {
        0
    }
}

fn build_tree_features(p: &RawPayload<'_>) -> Option<[f32; FEATURE_COUNT]> {
    let requested = parse_iso(p.requested_at)?;
    let safe_avg = p.customer_avg_amount.max(1.0);
    let amount_ratio = p.amount / safe_avg;
    let known = merchant_known(p);

    let (minutes_since_last, km_from_last, last_null) = match p.last_timestamp {
        Some(ts) => {
            let last = parse_iso(ts)?;
            let delta_seconds = requested.epoch_seconds - last.epoch_seconds;
            let mins = clamp01(delta_seconds.max(0) as f32 / 60.0 / MAX_MINUTES);
            let km = if let Some(km) = p.last_km {
                clamp01(km / MAX_KM)
            } else {
                -1.0
            };
            (mins, km, 0.0)
        }
        None => (-1.0, -1.0, 1.0),
    };

    Some([
        clamp01(p.amount / MAX_AMOUNT),
        clamp01(p.installments as f32 / MAX_INSTALLMENTS),
        clamp01(amount_ratio / AMOUNT_VS_AVG_RATIO),
        requested.hour as f32 / 23.0,
        requested.weekday_monday0 as f32 / 6.0,
        minutes_since_last,
        km_from_last,
        clamp01(p.km_from_home / MAX_KM),
        clamp01(p.tx_count_24h as f32 / MAX_TX24H),
        if p.is_online { 1.0 } else { 0.0 },
        if p.card_present { 1.0 } else { 0.0 },
        if known { 0.0 } else { 1.0 },
        mcc_risk_table(p.merchant_mcc),
        clamp01(p.merchant_avg_amount / MAX_MERCHANT_AVG),
        last_null,
        p.amount,
        p.customer_avg_amount,
        amount_ratio,
        p.tx_count_24h as f32,
        p.km_from_home,
        p.merchant_avg_amount,
    ])
}

struct ParsedTime {
    hour: u8,
    weekday_monday0: u8,
    epoch_seconds: i64,
}

fn parse_iso(ts: &[u8]) -> Option<ParsedTime> {
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
    let second = if ts.len() >= 19 {
        digit2(ts[17], ts[18])? as i64
    } else {
        0
    };
    let days = days_from_civil(year, month, day);
    let weekday = ((days + 3).rem_euclid(7)) as u8;
    Some(ParsedTime {
        hour: hour as u8,
        weekday_monday0: weekday,
        epoch_seconds: days * 86_400 + hour * 3600 + minute * 60 + second,
    })
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
fn merchant_known(p: &RawPayload<'_>) -> bool {
    contains_quoted(p.known_merchants, p.merchant_id)
}

#[inline]
fn contains_quoted(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() + 1 < haystack.len() {
        if haystack[i] == b'"'
            && haystack[i + 1..].starts_with(needle)
            && haystack.get(i + 1 + needle.len()) == Some(&b'"')
        {
            return true;
        }
        i += 1;
    }
    false
}

#[inline]
fn is_safe_mcc(mcc: &[u8]) -> bool {
    matches!(mcc, b"5411" | b"5812" | b"5912" | b"5311")
}

#[inline]
fn is_risky_mcc(mcc: &[u8]) -> bool {
    matches!(mcc, b"7995" | b"7801" | b"7802")
}

#[inline]
fn mcc_risk_table(mcc: &[u8]) -> f32 {
    match mcc {
        b"5411" => 0.15,
        b"5812" => 0.30,
        b"5912" => 0.20,
        b"5944" => 0.45,
        b"7801" => 0.80,
        b"7802" => 0.75,
        b"7995" => 0.85,
        b"4511" => 0.35,
        b"5311" => 0.25,
        b"5999" => 0.50,
        _ => 0.50,
    }
}
