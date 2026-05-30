//! Estatísticas de caminho do tier_score (atalhos vs árvore vs ratio).

use fraud_detector::ingest::extract_filled;
use fraud_detector::search::{ratio_only_count, tier_fraud_count, tier_path, tree_only_count, TierPath};
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

fn main() {
    let data_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let raw = fs::read_to_string(&data_path).unwrap();
    let file: TestFile = serde_json::from_str(&raw).unwrap();

    let mut legit = 0u64;
    let mut fraud = 0u64;
    let mut tree = 0u64;
    let mut ratio = 0u64;
    let mut tree_ratio_same = 0u64;
    let mut tree_no_last = 0u64;
    let mut tree_no_last_ratio_same = 0u64;

    for entry in &file.entries {
        let body = serde_json::to_vec(&entry.request).unwrap();
        let Some(p) = extract_filled(&body) else {
            continue;
        };
        match tier_path(&p) {
            TierPath::ObviousLegit => legit += 1,
            TierPath::ObviousFraud => fraud += 1,
            TierPath::Tree => {
                tree += 1;
                let tc = tree_only_count(&p).unwrap();
                let rc = ratio_only_count(&p);
                let ta = tc <= 2;
                let ra = rc <= 2;
                if ta == ra {
                    tree_ratio_same += 1;
                }
                if p.last_timestamp.is_none() {
                    tree_no_last += 1;
                    if ta == ra {
                        tree_no_last_ratio_same += 1;
                    }
                }
            }
            TierPath::Ratio => ratio += 1,
        }
        let _ = tier_fraud_count(&p);
    }

    let n = file.entries.len();
    eprintln!("entries={n}");
    eprintln!("obvious_legit={legit} ({:.1}%)", 100.0 * legit as f64 / n as f64);
    eprintln!("obvious_fraud={fraud} ({:.1}%)", 100.0 * fraud as f64 / n as f64);
    eprintln!("tree={tree} ({:.1}%)", 100.0 * tree as f64 / n as f64);
    eprintln!("ratio_fallback={ratio} ({:.1}%)", 100.0 * ratio as f64 / n as f64);
    if tree > 0 {
        eprintln!(
            "tree_vs_ratio_same={tree_ratio_same}/{tree} ({:.1}%)",
            100.0 * tree_ratio_same as f64 / tree as f64
        );
        eprintln!(
            "tree_no_last_tx={tree_no_last} ratio_same={tree_no_last_ratio_same}/{tree_no_last}"
        );
    }
}
