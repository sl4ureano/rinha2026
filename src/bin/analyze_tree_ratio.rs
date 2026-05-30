//! Mede quando ratio_count == tier_fraud_count no caminho tree (candidato a fast skip).

use fraud_detector::ingest::extract;
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

fn main() {
    let data_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let raw = fs::read_to_string(&data_path).unwrap();
    let file: TestFile = serde_json::from_str(&raw).unwrap();

    let mut ratio0_tree_ne5 = 0u64;
    let mut ratio0_tree_eq0 = 0u64;
    let mut ratio5_tree_ne0 = 0u64;
    let mut ratio5_tree_eq5 = 0u64;
    let mut ratio_matches_tier = 0u64;
    let mut tree_path = 0u64;

    // Candidato: ratio_count == tier → fast pode usar ratio sem árvore
    let mut ratio_eq_tier_on_tree = 0u64;
    let mut ratio_ne_tier_on_tree = 0u64;

    // Candidato soft: ratio extremo
    let mut extreme_low_ratio0_tier0 = 0u64;
    let mut extreme_low_ratio0_tier_ne0 = 0u64;
    let mut extreme_high_ratio5_tier5 = 0u64;
    let mut extreme_high_ratio5_tier_ne5 = 0u64;

    for entry in &file.entries {
        let body = serde_json::to_vec(&entry.request).unwrap();
        let Some(mut p) = extract(&body) else {
            continue;
        };
        if try_fast_fraud_count(&p).is_some() {
            continue;
        }
        complete_cache(&mut p);
        if tier_path(&p) != TierPath::Tree {
            continue;
        }
        tree_path += 1;

        let tier = tier_fraud_count(&p);
        let rc = p.cache.ratio_count;
        let tc = tree_only_count(&p).unwrap();

        if rc == tier {
            ratio_matches_tier += 1;
        }
        if rc == tier {
            ratio_eq_tier_on_tree += 1;
        } else {
            ratio_ne_tier_on_tree += 1;
        }

        if rc == 0 {
            if tc == 0 {
                ratio0_tree_eq0 += 1;
            } else {
                ratio0_tree_ne5 += 1;
            }
        } else if rc == 5 {
            if tc == 5 {
                ratio5_tree_eq5 += 1;
            } else {
                ratio5_tree_ne0 += 1;
            }
        }

        let safe_avg = p.customer_avg_amount.max(1.0);
        let norm = (p.amount / safe_avg / 10.0).clamp(0.0, 1.0);
        if rc == 0 && norm < 0.01 {
            if tier == 0 {
                extreme_low_ratio0_tier0 += 1;
            } else {
                extreme_low_ratio0_tier_ne0 += 1;
            }
        }
        if rc == 5 && norm > 0.5 {
            if tier == 5 {
                extreme_high_ratio5_tier5 += 1;
            } else {
                extreme_high_ratio5_tier_ne5 += 1;
            }
        }
    }

    eprintln!("tree_path (non-fast)={tree_path}");
    eprintln!("ratio==tier on tree path: {ratio_eq_tier_on_tree}/{tree_path} ({:.1}%)", 100.0 * ratio_eq_tier_on_tree as f64 / tree_path as f64);
    eprintln!("ratio0 & tree0: {ratio0_tree_eq0}  ratio0 & tree5: {ratio0_tree_ne5}");
    eprintln!("ratio5 & tree5: {ratio5_tree_eq5}  ratio5 & tree0: {ratio5_tree_ne0}");
    eprintln!(
        "extreme low norm<0.01 ratio0: tier0={extreme_low_ratio0_tier0} tier!=0={extreme_low_ratio0_tier_ne0}"
    );
    eprintln!(
        "extreme high norm>0.5 ratio5: tier5={extreme_high_ratio5_tier5} tier!=5={extreme_high_ratio5_tier_ne5}"
    );
}
