//! `build-index` entry logic.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::build::entry::entry_to_vector;
use crate::build::sources::{load_mcc_risk, load_references_gz};
use crate::build::{build_index_with_leaf, BuildInputs, DEFAULT_LEAF_SIZE};
use crate::index::{quantize_value, MCC_TABLE_SIZE};

pub fn run() {
    let args: Vec<String> = env::args().collect();
    let resources_dir = PathBuf::from(args.get(1).map(|s| s.as_str()).unwrap_or("resources"));
    let out_path = PathBuf::from(
        args.get(2)
            .map(|s| s.as_str())
            .unwrap_or("data/index.bin"),
    );
    let leaf_size: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_LEAF_SIZE);

    eprintln!("loading mcc risk from {}", resources_dir.display());
    let mcc_risk = load_mcc_risk(&resources_dir.join("mcc_risk.json"))
        .unwrap_or_else(|e| panic!("mcc_risk: {e}"));
    let mut mcc_table = [0i16; MCC_TABLE_SIZE];
    for (&mcc, &risk) in mcc_risk.iter() {
        let idx = (mcc as usize) % MCC_TABLE_SIZE;
        mcc_table[idx] = quantize_value(risk as f64);
    }

    let refs_path = resources_dir.join("references.json.gz");
    eprintln!("loading references from {}", refs_path.display());
    let entries = load_references_gz(&refs_path).unwrap_or_else(|e| panic!("references: {e}"));
    eprintln!("loaded {} reference entries", entries.len());

    let t = Instant::now();
    let mut vectors = Vec::with_capacity(entries.len());
    let mut labels = Vec::with_capacity(entries.len());
    for e in &entries {
        let (v, f) = entry_to_vector(e);
        vectors.push(v);
        labels.push(f);
    }
    drop(entries);
    eprintln!("quantized {} vectors in {:?}", vectors.len(), t.elapsed());

    let t = Instant::now();
    let index_bytes = build_index_with_leaf(
        &BuildInputs {
            vectors: &vectors,
            labels: &labels,
            mcc_table: &mcc_table,
        },
        leaf_size,
    );
    eprintln!(
        "built index ({} bytes, leaf={}) in {:?}",
        index_bytes.len(),
        leaf_size,
        t.elapsed()
    );

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&out_path, &index_bytes).unwrap_or_else(|e| panic!("write index: {e}"));
    eprintln!(
        "wrote {} ({:.1} MB)",
        out_path.display(),
        index_bytes.len() as f64 / 1_048_576.0
    );
}
