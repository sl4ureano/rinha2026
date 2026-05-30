//! Valida tier scorer vs expected_approved em test-data.json.

use fraud_detector::ingest::extract_filled;
use fraud_detector::search::tier_fraud_count;
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
    expected_approved: bool,
}

fn main() {
    let data_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let raw = fs::read_to_string(&data_path).unwrap_or_else(|e| {
        eprintln!("read {}: {e}", data_path.display());
        std::process::exit(1);
    });
    let file: TestFile = serde_json::from_str(&raw).unwrap();

    let mut fp = 0u64;
    let mut fn_ = 0u64;
    let mut parse_fail = 0u64;

    for entry in &file.entries {
        let body = serde_json::to_vec(&entry.request).expect("serialize");
        let Some(p) = extract_filled(&body) else {
            parse_fail += 1;
            continue;
        };
        let count = tier_fraud_count(&p);
        let approved = count <= 2;
        if approved && !entry.expected_approved {
            fn_ += 1;
        } else if !approved && entry.expected_approved {
            fp += 1;
        }
    }

    eprintln!(
        "entries={} fp={} fn={} parse_fail={}",
        file.entries.len(),
        fp,
        fn_,
        parse_fail
    );
    if fp > 0 || fn_ > 0 {
        std::process::exit(1);
    }
}
