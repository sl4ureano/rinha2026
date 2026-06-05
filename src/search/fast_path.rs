//! Atalhos parciais. O hot path híbrido só aprova gasto claramente seguro;
//! qualquer caso arriscado/cinza cai no k-NN exato.
//! Retorna `Some(count)` só quando o perfil encaixa; senão cai no k-NN.

use crate::ingest::RawPayload;

const MAX_AMOUNT_LEGIT: f32 = 500.0;
const MAX_RATIO_LEGIT: f32 = 0.5;
const MAX_INSTALLMENTS_LEGIT: u32 = 3;
const MAX_TX24H_LEGIT: u32 = 5;
const MAX_KM_HOME_LEGIT: f32 = 50.0;

/// Soma de labels dos top-5 vizinhos (0–5), igual a `fraud_count`, ou `None` → usar k-NN.
pub fn try_fast_fraud_count(p: &RawPayload<'_>) -> Option<u8> {
    try_fast_safe_count(p)
}

/// Aprovação rápida conservadora. Nunca nega sem consultar o k-NN.
#[inline]
pub fn try_fast_safe_count(p: &RawPayload<'_>) -> Option<u8> {
    if obvious_legit(p) {
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
fn merchant_known(p: &RawPayload<'_>) -> bool {
    p.cache.merchant_known
}

#[inline]
fn is_safe_mcc(mcc: u32) -> bool {
    matches!(mcc, 0x3534_3131 | 0x3538_3132 | 0x3539_3132 | 0x3533_3131)
}
