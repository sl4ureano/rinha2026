//! Gera perfil LLVM para PGO (`-Cprofile-generate`). Usa o mesmo caminho que produção.

use std::env;
use std::fs;
use std::path::PathBuf;

use fraud_detector::index::Index;
use fraud_detector::search::score_for_profile;

fn main() {
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("resources/example-payloads.json"));

    let iters: usize = env::var("PGO_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000);

    let index_path = env::var("INDEX_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/index.bin"));
    let index = Index::open(&index_path).unwrap_or_else(|_| Index::empty());

    let raw = fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("read {}: {e}", path.display());
        std::process::exit(1);
    });

    let values: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("parse {}: {e} (expected JSON array of requests)", path.display());
        std::process::exit(1);
    });

    let bodies: Vec<Vec<u8>> = values
        .iter()
        .map(|v| serde_json::to_vec(v).expect("serialize request"))
        .collect();

    if bodies.is_empty() {
        eprintln!("no payloads in {}", path.display());
        std::process::exit(1);
    }

    let mut acc = 0u64;
    for i in 0..iters {
        acc = acc.wrapping_add(score_for_profile(&index, &bodies[i % bodies.len()]));
    }

    eprintln!(
        "pgo-train: {iters} iterations, {} unique bodies, acc={acc}",
        bodies.len()
    );
}
