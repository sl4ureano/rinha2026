//! Boot warmup: prime CPU caches and branch predictors before accepting traffic.

use crate::ingest::extract;
use crate::search::{complete_cache, tier_gray_count, try_fast_fraud_count};

const DEFAULT_WARMUP: usize = 4096;

/// Representative payloads: obvious legit, obvious fraud, gray (tree/ratio).
const BODIES: &[&[u8]] = &[
    br#"{"id":"w1","transaction":{"amount":41.12,"installments":2,"requested_at":"2026-03-11T18:45:53Z"},"customer":{"avg_amount":82.24,"tx_count_24h":3,"known_merchants":["MERC-003","MERC-016"]},"merchant":{"id":"MERC-016","mcc":"5411","avg_amount":60.25},"terminal":{"is_online":false,"card_present":true,"km_from_home":29.2},"last_transaction":null}"#,
    br#"{"id":"w2","transaction":{"amount":9500.0,"installments":6,"requested_at":"2026-03-11T02:15:00Z"},"customer":{"avg_amount":120.0,"tx_count_24h":8,"known_merchants":[]},"merchant":{"id":"MERC-UNK","mcc":"7995","avg_amount":500.0},"terminal":{"is_online":true,"card_present":false,"km_from_home":200.0},"last_transaction":null}"#,
    br#"{"id":"w3","transaction":{"amount":1200.0,"installments":4,"requested_at":"2026-03-11T14:22:11Z"},"customer":{"avg_amount":400.0,"tx_count_24h":4,"known_merchants":["MERC-001"]},"merchant":{"id":"MERC-099","mcc":"5944","avg_amount":800.0},"terminal":{"is_online":false,"card_present":true,"km_from_home":80.0},"last_transaction":{"timestamp":"2026-03-11T13:00:00Z","km_from_last":12.5}}"#,
    br#"{"id":"w4","transaction":{"amount":250.0,"installments":1,"requested_at":"2026-03-11T09:00:00Z"},"customer":{"avg_amount":500.0,"tx_count_24h":2,"known_merchants":["MERC-010"]},"merchant":{"id":"MERC-010","mcc":"5812","avg_amount":45.0},"terminal":{"is_online":false,"card_present":true,"km_from_home":5.0},"last_transaction":null}"#,
];

fn warmup_count() -> usize {
    std::env::var("WARMUP_QUERIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_WARMUP)
}

/// Run `WARMUP_QUERIES` (default 4096) scoring iterations before serving.
pub fn run_warmup() {
    let n = warmup_count();
    if n == 0 {
        return;
    }
    let bodies = BODIES.len();
    for i in 0..n {
        let body = BODIES[i % bodies];
        if let Some(p) = extract(body) {
            let mut p = p;
            complete_cache(&mut p);
            if try_fast_fraud_count(&p).is_none() {
                let _ = tier_gray_count(&p);
            }
        }
    }
    eprintln!("warmup: {n} queries");
}
