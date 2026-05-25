//! Atalhos parciais (só gasto seguro / arriscado parcial); produção usa `tier_score`.
//! Retorna `Some(count)` só quando o perfil encaixa; senão cai no k-NN.

use crate::index::Index;
use crate::ingest::RawPayload;

const MAX_AMOUNT_LEGIT: f32 = 500.0;
const MAX_RATIO_LEGIT: f32 = 0.5;
const MAX_INSTALLMENTS_LEGIT: u32 = 3;
const MAX_TX24H_LEGIT: u32 = 5;
const MAX_KM_HOME_LEGIT: f32 = 50.0;

const MIN_AMOUNT_FRAUD: f32 = 5000.0;
const MIN_INSTALLMENTS_FRAUD: u32 = 5;
const MIN_TX24H_FRAUD: u32 = 6;
const MIN_KM_HOME_FRAUD: f32 = 150.0;

/// Soma de labels dos top-5 vizinhos (0–5), igual a `fraud_count`, ou `None` → usar k-NN.
pub fn try_fast_fraud_count(index: &Index, p: &RawPayload<'_>) -> Option<u8> {
    if obvious_legit(p) {
        return Some(0);
    }
    if obvious_fraud(index, p) {
        return Some(5);
    }
    None
}

#[inline]
fn obvious_legit(p: &RawPayload<'_>) -> bool {
    if p.amount > MAX_AMOUNT_LEGIT {
        return false;
    }
    let safe_avg = p.customer_avg_amount.max(1.0);
    let ratio = p.amount / safe_avg;
    if ratio > MAX_RATIO_LEGIT {
        return false;
    }
    if p.installments > MAX_INSTALLMENTS_LEGIT {
        return false;
    }
    if p.tx_count_24h > MAX_TX24H_LEGIT {
        return false;
    }
    if !merchant_known(p) {
        return false;
    }
    if p.km_from_home > MAX_KM_HOME_LEGIT {
        return false;
    }
    is_safe_mcc(p.merchant_mcc)
}

#[inline]
fn obvious_fraud(index: &Index, p: &RawPayload<'_>) -> bool {
    if p.amount < MIN_AMOUNT_FRAUD {
        return false;
    }
    if p.installments < MIN_INSTALLMENTS_FRAUD {
        return false;
    }
    if p.tx_count_24h < MIN_TX24H_FRAUD {
        return false;
    }
    if merchant_known(p) {
        return false;
    }
    if p.km_from_home < MIN_KM_HOME_FRAUD {
        return false;
    }
    if !is_risky_mcc(index, p.merchant_mcc) {
        return false;
    }
    true
}

#[inline]
fn merchant_known(p: &RawPayload<'_>) -> bool {
    contains_quoted(p.known_merchants, p.merchant_id)
}

#[inline]
fn contains_quoted(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() + 1 < haystack.len() {
        if haystack[i] == b'"'
            && haystack[i + 1..].starts_with(needle)
            && haystack.get(i + 1 + needle.len()) == Some(&b'"')
        {
            return true;
        }
        i += 1;
    }
    false
}

#[inline]
fn is_safe_mcc(mcc: &[u8]) -> bool {
    matches!(mcc, b"5411" | b"5812" | b"5912" | b"5311")
}

#[inline]
fn is_risky_mcc(index: &Index, mcc: &[u8]) -> bool {
    let mcc_num = parse_mcc(mcc);
    let risk = index.mcc_risk(mcc_num);
    // Alto risco no índice (gambling, cash advance, etc.)
    risk >= 7500
        || matches!(mcc, b"7995" | b"7801" | b"7802")
}

#[inline]
fn parse_mcc(mcc: &[u8]) -> u32 {
    let mut acc: u32 = 0;
    for &c in mcc {
        if !c.is_ascii_digit() {
            return 0;
        }
        acc = acc.saturating_mul(10).saturating_add((c - b'0') as u32);
    }
    acc
}
