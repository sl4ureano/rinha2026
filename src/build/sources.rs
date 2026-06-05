use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

pub fn load_mcc_risk(path: &Path) -> Result<HashMap<u32, f32>> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
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

pub fn load_references_gz(path: &Path) -> Result<Vec<ReferenceEntry>> {
    let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut decoder = flate2::read::GzDecoder::new(f);
    let mut buf = Vec::with_capacity(300 * 1024 * 1024);
    decoder.read_to_end(&mut buf)?;
    let entries: Vec<ReferenceEntry> = serde_json::from_slice(&buf)?;
    Ok(entries)
}
