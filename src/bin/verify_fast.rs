//! Valida fast-path vs k-NN exato em test-data.json (0 divergências obrigatório).

use fraud_detector::index::Index;
use fraud_detector::ingest::{extract, vectorize_payload};
use fraud_detector::search::{fraud_count, try_fast_fraud_count};
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

fn approved_from_count(count: u8) -> bool {
    count <= 2
}

fn main() {
    let index_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/index.bin"));
    let data_path = env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let index = Index::open(&index_path).unwrap_or_else(|e| {
        eprintln!("index open {}: {e}", index_path.display());
        std::process::exit(1);
    });

    let raw = fs::read_to_string(&data_path).unwrap_or_else(|e| {
        eprintln!("read {}: {e}", data_path.display());
        std::process::exit(1);
    });
    let file: TestFile = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("parse json: {e}");
        std::process::exit(1);
    });

    let mut fast_hits = 0u64;
    let mut mismatches = 0u64;
    let mut parse_fail = 0u64;

    for entry in &file.entries {
        let body = serde_json::to_vec(&entry.request).expect("serialize request");
        let Some(p) = extract(&body) else {
            parse_fail += 1;
            continue;
        };
        let Some(v) = vectorize_payload(&index, &p) else {
            parse_fail += 1;
            continue;
        };
        let exact = fraud_count(&index, &v);
        let exact_ok = approved_from_count(exact);

        if let Some(fast) = try_fast_fraud_count(&p) {
            fast_hits += 1;
            let fast_ok = approved_from_count(fast);
            if fast_ok != exact_ok {
                mismatches += 1;
                if mismatches <= 5 {
                    eprintln!(
                        "mismatch fast={fast} exact={exact} amount={} tx24h={}",
                        p.amount, p.tx_count_24h
                    );
                }
            }
        }
    }

    eprintln!(
        "entries={} fast_hits={} mismatches={} parse_fail={}",
        file.entries.len(),
        fast_hits,
        mismatches,
        parse_fail
    );
    if mismatches > 0 {
        std::process::exit(1);
    }
}
