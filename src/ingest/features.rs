use crate::index::{vectorize, Index, QueryVector, RawFeatures};
use crate::ingest::RawPayload;

pub fn vectorize_payload(index: &Index, p: &RawPayload<'_>) -> Option<QueryVector> {
    if !p.cache.requested_valid {
        return None;
    }
    let mcc = parse_ascii_u32(p.merchant_mcc)?;
    let unknown = !p.cache.merchant_known;

    let mcc_risk_q = mcc_risk_quantized(index, mcc);

    let minutes_since_last_tx: Option<u32> = if p.cache.last_present {
        if !p.cache.last_epoch_ok {
            return None;
        }
        Some(((p.cache.req_epoch - p.cache.last_epoch) / 60).max(0) as u32)
    } else {
        None
    };

    let raw = RawFeatures {
        amount_milli: to_milli(p.amount),
        installments: p.installments,
        hour_of_day: p.cache.req_hour,
        day_of_week: p.cache.req_weekday,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_milli_round_trip() {
        assert_eq!(to_milli(0.0), 0);
        assert_eq!(to_milli(1.5), 1500);
        assert_eq!(to_milli(41.12), 41120);
        assert_eq!(to_milli(-3.0), 0);
    }
}
