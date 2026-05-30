//! Scorer em camadas: gasto seguro → gasto arriscado → árvore → ratio (sem k-NN em runtime).

use crate::ingest::RawPayload;
use crate::search::decision_tree::{self, FEATURE_COUNT};

const MAX_AMOUNT: f32 = 10_000.0;
const MAX_INSTALLMENTS: f32 = 12.0;
const AMOUNT_VS_AVG_RATIO: f32 = 10.0;
const MAX_MINUTES: f32 = 1440.0;
const MAX_KM: f32 = 1000.0;
const MAX_TX24H: f32 = 20.0;
const MAX_MERCHANT_AVG: f32 = 10_000.0;
const LEGIT_RATIO_CAP: f32 = 0.50001;

/// Caminho tomado (útil para `tier_paths` / tuning).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TierPath {
    ObviousLegit,
    ObviousFraud,
    Tree,
    Ratio,
}

struct TierCtx {
    safe_avg: f32,
    known: bool,
    mcc: u32,
    requested: Option<ParsedTime>,
}

impl TierCtx {
    #[inline]
    fn from_payload(p: &RawPayload<'_>) -> Self {
        let c = &p.cache;
        Self {
            safe_avg: p.customer_avg_amount.max(1.0),
            known: c.merchant_known,
            mcc: c.mcc_u32,
            requested: if c.requested_valid {
                Some(ParsedTime {
                    hour: c.req_hour,
                    weekday_monday0: c.req_weekday,
                    epoch_seconds: c.req_epoch,
                })
            } else {
                None
            },
        }
    }
}

/// Gray area only: tree → ratio. Caller must have already run `try_fast_fraud_count`.
#[inline]
pub fn tier_gray_count(p: &RawPayload<'_>) -> u8 {
    if p.cache.tree_ready {
        return if decision_tree::predict(&p.cache.tree_features) {
            5
        } else {
            0
        };
    }
    p.cache.ratio_count
}

/// Build tree features into `p.cache` (call once per request after `extract`).
#[inline]
pub fn complete_cache(p: &mut RawPayload<'_>) {
    if p.cache.gray_ratio_only || !p.cache.requested_valid {
        return;
    }
    if let Some(features) = build_tree_features(p) {
        p.cache.tree_features = features;
        p.cache.tree_ready = true;
    }
}

/// Contagem 0–5 para respostas HTTP estáticas (0 = aprova, 5 = nega).
#[inline]
pub fn tier_fraud_count(p: &RawPayload<'_>) -> u8 {
    let ctx = TierCtx::from_payload(p);
    if obvious_legit(p, &ctx) {
        return 0;
    }
    if obvious_fraud(p, &ctx) {
        return 5;
    }
    score_gray(p, &ctx)
}

#[inline]
fn score_gray(p: &RawPayload<'_>, ctx: &TierCtx) -> u8 {
    if ctx.requested.is_none() {
        return p.cache.ratio_count;
    }
    if p.cache.tree_ready {
        return if decision_tree::predict(&p.cache.tree_features) {
            5
        } else {
            0
        };
    }
    if let Some(features) = build_tree_features(p) {
        return if decision_tree::predict(&features) { 5 } else { 0 };
    }
    p.cache.ratio_count
}

#[inline]
pub fn tier_path(p: &RawPayload<'_>) -> TierPath {
    let ctx = TierCtx::from_payload(p);
    if obvious_legit(p, &ctx) {
        return TierPath::ObviousLegit;
    }
    if obvious_fraud(p, &ctx) {
        return TierPath::ObviousFraud;
    }
    if ctx.requested.is_none() {
        return TierPath::Ratio;
    }
    if build_tree_features(p).is_some() {
        TierPath::Tree
    } else {
        TierPath::Ratio
    }
}

#[inline]
fn obvious_legit(p: &RawPayload<'_>, ctx: &TierCtx) -> bool {
    if p.amount > 500.0 {
        return false;
    }
    if p.amount > ctx.safe_avg * LEGIT_RATIO_CAP {
        return false;
    }
    if p.installments > 3 {
        return false;
    }
    if p.tx_count_24h > 5 {
        return false;
    }
    if p.km_from_home > 50.0 {
        return false;
    }
    if !mcc_is_safe(ctx.mcc) {
        return false;
    }
    ctx.known
}

#[inline]
fn obvious_fraud(p: &RawPayload<'_>, ctx: &TierCtx) -> bool {
    p.amount >= 5000.0
        && p.installments >= 5
        && p.tx_count_24h >= 6
        && p.km_from_home >= 150.0
        && mcc_is_risky(ctx.mcc)
        && !ctx.known
}

/// Só a árvore (para análise offline em `tier-paths`).
pub fn tree_only_count(p: &RawPayload<'_>) -> Option<u8> {
    let features = build_tree_features(p)?;
    Some(if decision_tree::predict(&features) {
        5
    } else {
        0
    })
}

/// Só o ratio (para análise offline em `tier-paths`).
pub fn ratio_only_count(p: &RawPayload<'_>) -> u8 {
    p.cache.ratio_count
}

fn build_tree_features(p: &RawPayload<'_>) -> Option<[f32; FEATURE_COUNT]> {
    let c = &p.cache;
    if !c.requested_valid {
        return None;
    }
    let safe_avg = p.customer_avg_amount.max(1.0);
    let amount_ratio = p.amount / safe_avg;

    let (minutes_since_last, km_from_last, last_null) = if !c.last_present {
        (-1.0, -1.0, 1.0)
    } else if c.last_epoch_ok {
        let delta_seconds = c.req_epoch - c.last_epoch;
        let mins = clamp01(delta_seconds.max(0) as f32 / 60.0 / MAX_MINUTES);
        let km = if let Some(km) = p.last_km {
            clamp01(km / MAX_KM)
        } else {
            -1.0
        };
        (mins, km, 0.0)
    } else {
        return None;
    };

    Some([
        clamp01(p.amount / MAX_AMOUNT),
        clamp01(p.installments as f32 / MAX_INSTALLMENTS),
        clamp01(amount_ratio / AMOUNT_VS_AVG_RATIO),
        c.req_hour as f32 / 23.0,
        c.req_weekday as f32 / 6.0,
        minutes_since_last,
        km_from_last,
        clamp01(p.km_from_home / MAX_KM),
        clamp01(p.tx_count_24h as f32 / MAX_TX24H),
        if p.is_online { 1.0 } else { 0.0 },
        if p.card_present { 1.0 } else { 0.0 },
        if c.merchant_known { 0.0 } else { 1.0 },
        mcc_risk_table_u32(c.mcc_u32),
        clamp01(p.merchant_avg_amount / MAX_MERCHANT_AVG),
        last_null,
        p.amount,
        p.customer_avg_amount,
        amount_ratio,
        p.tx_count_24h as f32,
        p.km_from_home,
        p.merchant_avg_amount,
    ])
}

#[derive(Copy, Clone)]
struct ParsedTime {
    hour: u8,
    weekday_monday0: u8,
    epoch_seconds: i64,
}

#[inline]
fn mcc_is_safe(mcc: u32) -> bool {
    matches!(mcc, 0x3534_3131 | 0x3538_3132 | 0x3539_3132 | 0x3533_3131)
}

#[inline]
fn mcc_is_risky(mcc: u32) -> bool {
    matches!(mcc, 0x3739_3935 | 0x3738_3031 | 0x3738_3032)
}

#[inline]
fn mcc_risk_table_u32(mcc: u32) -> f32 {
    match mcc {
        0x3534_3131 => 0.15,
        0x3538_3132 => 0.30,
        0x3539_3132 => 0.20,
        0x3539_3434 => 0.45,
        0x3738_3031 => 0.80,
        0x3738_3032 => 0.75,
        0x3739_3935 => 0.85,
        0x3435_3131 => 0.35,
        0x3533_3131 => 0.25,
        0x3539_3939 => 0.50,
        _ => 0.50,
    }
}

#[inline]
fn clamp01(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}
