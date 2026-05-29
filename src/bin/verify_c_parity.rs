//! Lista índices onde tier Rust (serde) != tier C (tier_one).

use fraud_detector::ingest::extract;
use fraud_detector::search::tier_fraud_count;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(serde::Deserialize)]
struct TestFile {
    entries: Vec<TestEntry>,
}

#[derive(serde::Deserialize)]
struct TestEntry {
    request: serde_json::Value,
}

fn c_tier(tier_one: &PathBuf, body: &[u8]) -> Option<u8> {
    let mut child = Command::new(tier_one)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(body).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim();
    s.parse().ok()
}

fn main() {
    let Some(tier_one) = env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: verify-c-parity <tier_one> [test-data.json]");
        eprintln!("build tier_one from https://github.com/adsanla/rinha2026");
        std::process::exit(1);
    };

    let data_path = env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let raw = fs::read_to_string(&data_path).unwrap();
    let file: TestFile = serde_json::from_str(&raw).unwrap();

    let mut diffs = 0u64;
    for (i, entry) in file.entries.iter().enumerate() {
        let body = serde_json::to_vec(&entry.request).unwrap();
        let rust_c = tier_fraud_count(extract(&body).as_ref().unwrap());
        let Some(c_c) = c_tier(&tier_one, &body) else {
            eprintln!("tier_one failed at {i}");
            continue;
        };
        if rust_c != c_c {
            diffs += 1;
            if diffs <= 5 {
                eprintln!("idx={i} rust={rust_c} c={c_c}");
            }
        }
    }
    eprintln!("diffs={diffs} / {}", file.entries.len());
}
