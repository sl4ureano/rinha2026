//! Verifies the gateway KNN path without fixture lookup.
//!
//! This tool may read payloads from a fixture-shaped file, but it ignores
//! `id`, `expected_*`, and any precomputed answer. Expected counts are
//! recomputed by brute force over the official reference vectors.

use fraud_detector::index::{quantize_value, Index, QueryVector, PACKED_DIMS, TOP_K, VECTOR_DIM};
use fraud_detector::ingest::{extract, vectorize_payload};
use fraud_detector::search::fraud_count;
use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct ReferenceEntry {
    vector: Vec<f32>,
    label: String,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let index_path = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/index.bin"));
    let payloads_path = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("resources/example-payloads.json"));
    let refs_path = args
        .get(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("resources/references.json.gz"));
    let limit = args.get(4).and_then(|s| s.parse::<usize>().ok());

    let index = Index::open(&index_path).unwrap_or_else(|e| {
        eprintln!("index open {}: {e}", index_path.display());
        std::process::exit(1);
    });
    let payloads = load_payloads(&payloads_path).unwrap_or_else(|e| {
        eprintln!("payloads {}: {e}", payloads_path.display());
        std::process::exit(1);
    });
    let refs = load_references(&refs_path).unwrap_or_else(|e| {
        eprintln!("references {}: {e}", refs_path.display());
        std::process::exit(1);
    });

    let total = limit.unwrap_or(payloads.len()).min(payloads.len());
    let mut mismatches = 0usize;
    let mut parse_fail = 0usize;

    for (i, payload) in payloads.iter().take(total).enumerate() {
        let body = serde_json::to_vec(payload).expect("serialize payload");
        let Some(parsed) = extract(&body) else {
            parse_fail += 1;
            continue;
        };
        let Some(query) = vectorize_payload(&index, &parsed) else {
            parse_fail += 1;
            continue;
        };

        let hot = fraud_count(&index, &query);
        let expected = brute_force_count(&refs, &query);
        if hot != expected {
            mismatches += 1;
            if mismatches <= 8 {
                eprintln!("mismatch idx={i} hot={hot} brute={expected} payload={payload}");
            }
        }
    }

    eprintln!(
        "checked={} refs={} mismatches={} parse_fail={}",
        total,
        refs.len(),
        mismatches,
        parse_fail
    );

    if mismatches > 0 || parse_fail > 0 {
        std::process::exit(1);
    }
}

fn load_payloads(path: &Path) -> anyhow::Result<Vec<Value>> {
    let raw = fs::read(path)?;
    let root: Value = serde_json::from_slice(&raw)?;

    if let Some(entries) = root.get("entries").and_then(Value::as_array) {
        let mut payloads = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(request) = entry.get("request") {
                payloads.push(request.clone());
            }
        }
        return Ok(payloads);
    }

    if let Some(arr) = root.as_array() {
        return Ok(arr.clone());
    }

    Ok(vec![root])
}

fn load_references(path: &Path) -> anyhow::Result<Vec<(QueryVector, u8)>> {
    let file = fs::File::open(path)?;
    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf)?;
    let entries: Vec<ReferenceEntry> = serde_json::from_slice(&buf)?;

    Ok(entries
        .into_iter()
        .map(|entry| {
            let mut v = [0i16; PACKED_DIMS];
            for (d, value) in entry.vector.iter().take(VECTOR_DIM).enumerate() {
                v[d] = quantize_value(*value as f64);
            }
            let label = u8::from(entry.label == "fraud");
            (v, label)
        })
        .collect())
}

fn brute_force_count(refs: &[(QueryVector, u8)], query: &QueryVector) -> u8 {
    let mut best_dist = [i64::MAX; TOP_K];
    let mut best_label = [0u8; TOP_K];

    for (v, label) in refs {
        let d = squared_distance(query, v);
        if d < best_dist[TOP_K - 1] {
            insert_best(d, *label, &mut best_dist, &mut best_label);
        }
    }

    best_label.iter().sum()
}

fn squared_distance(a: &QueryVector, b: &QueryVector) -> i64 {
    let mut acc = 0i64;
    for d in 0..VECTOR_DIM {
        let diff = a[d] as i64 - b[d] as i64;
        acc += diff * diff;
    }
    acc
}

fn insert_best(dist: i64, label: u8, dists: &mut [i64; TOP_K], labels: &mut [u8; TOP_K]) {
    let mut pos = TOP_K - 1;
    while pos > 0 && dist < dists[pos - 1] {
        dists[pos] = dists[pos - 1];
        labels[pos] = labels[pos - 1];
        pos -= 1;
    }
    dists[pos] = dist;
    labels[pos] = label;
}
