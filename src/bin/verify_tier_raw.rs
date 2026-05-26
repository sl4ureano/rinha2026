//! Compara tier no JSON bruto do arquivo vs corpo re-serializado (como k6 / verify-tier).

use fraud_detector::ingest::extract;
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

fn extract_request_raw(entry_json: &str) -> Option<&str> {
    let k = entry_json.find("\"request\"")?;
    let obj = entry_json[k..].find('{')? + k;
    let rest = &entry_json[obj..];
    let mut depth = 0i32;
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn main() {
    let data_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let raw = fs::read_to_string(&data_path).unwrap();
    let file: TestFile = serde_json::from_str(&raw).unwrap();

    let entries_start = raw.find("\"entries\"").unwrap();
    let arr_start = raw[entries_start..].find('[').unwrap() + entries_start + 1;
    let mut pos = arr_start;
    let mut raw_vs_ser = 0u64;
    let mut fp_ser = 0u64;
    let mut fn_ser = 0u64;
    let mut fp_raw = 0u64;
    let mut fn_raw = 0u64;

    for entry in &file.entries {
        while pos < raw.len() && raw.as_bytes()[pos].is_ascii_whitespace() || raw.as_bytes()[pos] == b',' {
            pos += 1;
        }
        if raw.as_bytes().get(pos) == Some(&b']') {
            break;
        }
        let estart = pos;
        let mut depth = 0i32;
        let mut eend = estart;
        for (i, c) in raw[estart..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        eend = estart + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let entry_slice = &raw[estart..eend];
        pos = eend;

        let body_ser = serde_json::to_vec(&entry.request).unwrap();
        let body_raw = extract_request_raw(entry_slice)
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();

        let c_ser = tier_fraud_count(extract(&body_ser).as_ref().unwrap());
        let c_raw = tier_fraud_count(extract(&body_raw).as_ref().unwrap());

        if c_ser != c_raw {
            raw_vs_ser += 1;
        }

        let exp = entry.expected_approved;
        let approved_ser = c_ser <= 2;
        let approved_raw = c_raw <= 2;
        if approved_ser && !exp {
            fn_ser += 1;
        } else if !approved_ser && exp {
            fp_ser += 1;
        }
        if approved_raw && !exp {
            fn_raw += 1;
        } else if !approved_raw && exp {
            fp_raw += 1;
        }
    }

    eprintln!(
        "raw_vs_ser_tier_diff={} fp_ser={} fn_ser={} fp_raw={} fn_raw={}",
        raw_vs_ser, fp_ser, fn_ser, fp_raw, fn_raw
    );
}
