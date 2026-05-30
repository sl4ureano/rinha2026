//! Explora regras extras de fast_path vs tier_fraud_count (test-data.json).

use fraud_detector::ingest::extract_filled;
use fraud_detector::search::{
    complete_cache, tier_fraud_count, tier_path, tree_only_count, try_fast_fraud_count, TierPath,
};
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
    let n = file.entries.len();

    let mut fast_hits = 0u64;
    let mut tree_path = 0u64;
    let mut tree_eq_ratio = 0u64;
    let mut tree_ne_ratio = 0u64;
    let mut soft_legit_candidates = 0u64;
    let mut soft_legit_ok = 0u64;
    let mut soft_fraud_candidates = 0u64;
    let mut soft_fraud_ok = 0u64;
    let mut ratio_tree_agree_tier = 0u64;

    for entry in &file.entries {
        let body = serde_json::to_vec(&entry.request).unwrap();
        let Some(mut p) = extract_filled(&body) else {
            continue;
        };
        complete_cache(&mut p);

        let tier = tier_fraud_count(&p);
        if try_fast_fraud_count(&p).is_some() {
            fast_hits += 1;
            continue;
        }

        let path = tier_path(&p);
        if path == TierPath::Tree {
            tree_path += 1;
            let tc = tree_only_count(&p).unwrap();
            let rc = p.cache.ratio_count;
            if tc == rc {
                tree_eq_ratio += 1;
                if approved(tc) == approved(tier) && tc == tier {
                    ratio_tree_agree_tier += 1;
                }
            } else {
                tree_ne_ratio += 1;
            }
        }

        // Candidato: known + amount<=800 + installments<=4 + tx24h<=6 + km<=80 + safe mcc → tier=0?
        if soft_legit_rule(&p) {
            soft_legit_candidates += 1;
            if tier == 0 {
                soft_legit_ok += 1;
            }
        }

        // Candidato: !known + amount>=3000 + installments>=4 + tx24h>=5 + km>=100 + risky mcc → tier=5?
        if soft_fraud_rule(&p) {
            soft_fraud_candidates += 1;
            if tier == 5 {
                soft_fraud_ok += 1;
            }
        }
    }

    eprintln!("entries={n}");
    eprintln!(
        "fast_hits={fast_hits} ({:.1}%)",
        100.0 * fast_hits as f64 / n as f64
    );
    eprintln!(
        "tree_path={tree_path} tree==ratio={tree_eq_ratio} tree!=ratio={tree_ne_ratio}"
    );
    if tree_path > 0 {
        eprintln!(
            "  tree==ratio & matches tier: {ratio_tree_agree_tier}/{tree_path} ({:.1}%)",
            100.0 * ratio_tree_agree_tier as f64 / tree_path as f64
        );
    }
    eprintln!(
        "soft_legit rule: {soft_legit_ok}/{soft_legit_candidates} correct ({:.2}% precision if all applied)",
        if soft_legit_candidates > 0 {
            100.0 * soft_legit_ok as f64 / soft_legit_candidates as f64
        } else {
            0.0
        }
    );
    eprintln!(
        "soft_fraud rule: {soft_fraud_ok}/{soft_fraud_candidates} correct ({:.2}% precision)",
        if soft_fraud_candidates > 0 {
            100.0 * soft_fraud_ok as f64 / soft_fraud_candidates as f64
        } else {
            0.0
        }
    );
}

fn soft_legit_rule(p: &fraud_detector::ingest::RawPayload<'_>) -> bool {
    let c = &p.cache;
    c.merchant_known
        && p.amount <= 800.0
        && p.installments <= 4
        && p.tx_count_24h <= 6
        && p.km_from_home <= 80.0
        && matches!(
            c.mcc_u32,
            0x3534_3131 | 0x3538_3132 | 0x3539_3132 | 0x3533_3131
        )
}

fn soft_fraud_rule(p: &fraud_detector::ingest::RawPayload<'_>) -> bool {
    let c = &p.cache;
    !c.merchant_known
        && p.amount >= 3000.0
        && p.installments >= 4
        && p.tx_count_24h >= 5
        && p.km_from_home >= 100.0
        && matches!(c.mcc_u32, 0x3739_3935 | 0x3738_3031 | 0x3738_3032)
}
