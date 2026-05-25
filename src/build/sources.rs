use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Normalization {
    pub max_amount: f32,
    pub max_installments: u32,
    pub amount_vs_avg_ratio: f32,
    pub max_minutes: f32,
    pub max_km: f32,
    pub max_tx_count_24h: u32,
    pub max_merchant_avg_amount: f32,
}

pub fn load_normalization(path: &Path) -> Result<Normalization> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let n: Normalization = serde_json::from_slice(&bytes)?;
    Ok(n)
}

pub fn load_mcc_risk(path: &Path) -> Result<HashMap<u32, f32>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let raw: HashMap<String, f32> = serde_json::from_slice(&bytes)?;
    Ok(raw
        .into_iter()
        .filter_map(|(k, v)| k.parse::<u32>().ok().map(|key| (key, v)))
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct ReferenceEntry {
    pub vector: Vec<f32>,
    pub label: String,
}

pub fn load_references_json(path: &Path) -> Result<Vec<ReferenceEntry>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let entries: Vec<ReferenceEntry> = serde_json::from_slice(&bytes)?;
    Ok(entries)
}

pub fn load_references_gz(path: &Path) -> Result<Vec<ReferenceEntry>> {
    let f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut decoder = flate2::read::GzDecoder::new(f);
    let mut buf = Vec::with_capacity(300 * 1024 * 1024);
    decoder.read_to_end(&mut buf)?;
    let entries: Vec<ReferenceEntry> = serde_json::from_slice(&buf)?;
    Ok(entries)
}
