//! Emite uma linha por entrada: tier_fraud_count no JSON bruto do arquivo.

use fraud_detector::ingest::extract;
use fraud_detector::search::tier_fraud_count;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

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
    let entries_start = raw.find("\"entries\"").unwrap();
    let arr_start = raw[entries_start..].find('[').unwrap() + entries_start + 1;
    let mut pos = arr_start;
    let mut out = io::stdout().lock();

    loop {
        while pos < raw.len() && (raw.as_bytes()[pos].is_ascii_whitespace() || raw.as_bytes()[pos] == b',') {
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

        let body = extract_request_raw(entry_slice).unwrap().as_bytes();
        let count = tier_fraud_count(extract(body).as_ref().unwrap());
        writeln!(out, "{count}").unwrap();
    }
}
