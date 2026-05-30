//! Atalhos parciais (só gasto seguro / arriscado parcial); produção usa `tier_score`.
//! Retorna `Some(count)` só quando o perfil encaixa; senão cai no k-NN.

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
pub fn try_fast_fraud_count(p: &RawPayload<'_>) -> Option<u8> {
    if obvious_legit(p) {
        return Some(0);
    }
    if obvious_fraud(p) {
        return Some(5);
    }
    if p.cache.gray_ratio_only {
        return Some(p.cache.ratio_count);
    }
    // Offline verify (analyze-tree-split): tree5=160, tree0=0 on ratio=5 tree path.
    if soft_tree_fraud(p) {
        return Some(5);
    }
    // Offline verify: tree0=10, tree5=0 (known, in-person, non-safe mcc, moderate amount).
    if soft_tree_legit(p) {
        return Some(0);
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
    is_safe_mcc(p.cache.mcc_u32)
}

#[inline]
fn obvious_fraud(p: &RawPayload<'_>) -> bool {
    if p.amount < MIN_AMOUNT_FRAUD {
        return false;
    }
    if p.installments < MIN_INSTALLMENTS_FRAUD {
        return false;
    }
    if p.tx_count_24h < MIN_TX24H_FRAUD {
        return false;
    }
    if p.cache.merchant_known {
        return false;
    }
    if p.km_from_home < MIN_KM_HOME_FRAUD {
        return false;
    }
    if !is_risky_mcc(p.cache.mcc_u32) {
        return false;
    }
    true
}

#[inline]
fn merchant_known(p: &RawPayload<'_>) -> bool {
    p.cache.merchant_known
}

#[inline]
fn is_safe_mcc(mcc: u32) -> bool {
    matches!(mcc, 0x3534_3131 | 0x3538_3132 | 0x3539_3132 | 0x3533_3131)
}

#[inline]
fn is_risky_mcc(mcc: u32) -> bool {
    matches!(mcc, 0x3739_3935 | 0x3738_3031 | 0x3738_3032)
}

/// ratio=5 tree path: árvore sempre nega (160 amostras, 0 falsos positivos offline).
#[inline]
fn soft_tree_fraud(p: &RawPayload<'_>) -> bool {
    let c = &p.cache;
    !c.merchant_known
        && !p.is_online
        && !p.card_present
        && is_risky_mcc(c.mcc_u32)
        && p.amount >= 2000.0
        && p.installments >= 3
        && p.km_from_home >= 200.0
}

/// ratio=5 tree path: árvore sempre aprova (10 amostras, 0 falsos negativos offline).
#[inline]
fn soft_tree_legit(p: &RawPayload<'_>) -> bool {
    let c = &p.cache;
    c.merchant_known
        && !p.is_online
        && p.card_present
        && !is_safe_mcc(c.mcc_u32)
        && p.amount <= 1500.0
        && p.installments <= 5
        && p.km_from_home <= 80.0
}
