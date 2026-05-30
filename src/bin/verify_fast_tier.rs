//! Garante try_fast_fraud_count ⊆ tier_fraud_count (0 divergências de aprovação).

use fraud_detector::ingest::extract;
use fraud_detector::search::{complete_cache, tier_fraud_count, try_fast_fraud_count};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(serde::Deserialize)]
struct TestFile {
    entries: Vec<TestEntry>,
}

#[derive(serde::Deserialize)]
struct TestEntry {
    request: serde_json::Value,
}

fn approved(count: u8) -> bool {
    count <= 2
}

fn main() {
    let data_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let raw = fs::read_to_string(&data_path).unwrap();
    let file: TestFile = serde_json::from_str(&raw).unwrap();

    let mut fast_hits = 0u64;
    let mut mismatches = 0u64;

    for entry in &file.entries {
        let body = serde_json::to_vec(&entry.request).unwrap();
        let Some(mut p) = extract(&body) else {
            continue;
        };
        complete_cache(&mut p);

        let tier = tier_fraud_count(&p);
        if let Some(fast) = try_fast_fraud_count(&p) {
            fast_hits += 1;
            if approved(fast) != approved(tier) {
                mismatches += 1;
                if mismatches <= 5 {
                    eprintln!("mismatch fast={fast} tier={tier} amount={}", p.amount);
                }
            }
        }
    }

    eprintln!(
        "entries={} fast_hits={} mismatches={}",
        file.entries.len(),
        fast_hits,
        mismatches
    );
    if mismatches > 0 {
        std::process::exit(1);
    }
}
