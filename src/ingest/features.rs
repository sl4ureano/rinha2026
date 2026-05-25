use crate::index::{vectorize, Index, QueryVector, RawFeatures};
use crate::ingest::RawPayload;

pub fn vectorize_payload(index: &Index, p: &RawPayload<'_>) -> Option<QueryVector> {
    let req_minutes = parse_iso8601_minutes(p.requested_at)?;
    let (hour, dow) = hour_and_dow_from_minutes(req_minutes);
    let mcc = parse_ascii_u32(p.merchant_mcc)?;
    let unknown = !contains_quoted(p.known_merchants, p.merchant_id);

    let mcc_risk_q = mcc_risk_quantized(index, mcc);

    let minutes_since_last_tx: Option<u32> = match p.last_timestamp {
        Some(ts) => parse_iso8601_minutes(ts).map(|last| (req_minutes - last).max(0) as u32),
        None => None,
    };

    let raw = RawFeatures {
        amount_milli: to_milli(p.amount),
        installments: p.installments,
        hour_of_day: hour,
        day_of_week: dow,
        minutes_since_last_tx,
        km_from_last_tx_milli: p.last_km.map(to_milli),
        km_from_home_milli: to_milli(p.km_from_home),
        customer_avg_amount_milli: to_milli(p.customer_avg_amount),
        tx_count_24h: p.tx_count_24h,
        is_online: p.is_online,
        card_present: p.card_present,
        unknown_merchant: unknown,
        mcc_risk_q,
        merchant_avg_amount_milli: to_milli(p.merchant_avg_amount),
    };
    Some(vectorize(&raw))
}

#[inline]
fn to_milli(v: f32) -> u32 {
    if v <= 0.0 {
        return 0;
    }
    let scaled = (v as f64 * 1000.0).round();
    if scaled >= u32::MAX as f64 {
        u32::MAX
    } else {
        scaled as u32
    }
}

/// Look up MCC risk directly from the blob's lookup table. Builder writes
/// these as i16 in `[0, QUANT_SCALE]`, so this is a free read.
#[inline]
fn mcc_risk_quantized(index: &Index, mcc: u32) -> i16 {
    index.mcc_risk(mcc)
}

#[inline]
fn parse_ascii_u32(s: &[u8]) -> Option<u32> {
    let mut acc: u32 = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            return None;
        }
        acc = acc.checked_mul(10)?.checked_add((c - b'0') as u32)?;
    }
    Some(acc)
}

#[inline]
fn parse_iso8601_minutes(ts: &[u8]) -> Option<i64> {
    if ts.len() < 19 {
        return None;
    }
    // Fixed layout: YYYY-MM-DDTHH:MM:SS…
    let year = digit4(ts[0], ts[1], ts[2], ts[3])? as i64;
    if ts[4] != b'-' || ts[7] != b'-' || ts[10] != b'T' || ts[13] != b':' {
        return None;
    }
    let month = digit2(ts[5], ts[6])? as i64;
    let day = digit2(ts[8], ts[9])? as i64;
    let hour = digit2(ts[11], ts[12])? as i64;
    let minute = digit2(ts[14], ts[15])? as i64;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let days = days_from_civil(year, month as u32, day as u32);
    Some(days * 1440 + hour * 60 + minute)
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
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m_adj: u64 = if m > 2 {
        (m - 3) as u64
    } else {
        (m + 9) as u64
    };
    let doy = (153 * m_adj + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

#[inline]
fn hour_and_dow_from_minutes(total_minutes: i64) -> (u8, u8) {
    let mins_in_day = total_minutes.rem_euclid(1440);
    let hour = (mins_in_day / 60) as u8;
    let days = total_minutes.div_euclid(1440);
    let dow = ((days + 3).rem_euclid(7)) as u8;
    (hour, dow)
}

#[inline]
fn contains_quoted(haystack: &[u8], needle: &[u8]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeller_2026_03_11_is_wednesday() {
        let mins = parse_iso8601_minutes(b"2026-03-11T18:45:53Z").unwrap();
        let (_, dow) = hour_and_dow_from_minutes(mins);
        assert_eq!(dow, 2);
    }

    #[test]
    fn delta_minutes_correct() {
        let a = parse_iso8601_minutes(b"2026-03-11T20:00:00Z").unwrap();
        let b = parse_iso8601_minutes(b"2026-03-11T18:00:00Z").unwrap();
        assert_eq!(a - b, 120);
    }

    #[test]
    fn to_milli_round_trip() {
        assert_eq!(to_milli(0.0), 0);
        assert_eq!(to_milli(1.5), 1500);
        assert_eq!(to_milli(41.12), 41120);
        assert_eq!(to_milli(-3.0), 0);
    }
}
