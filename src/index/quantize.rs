//! Quantização e layout do índice vetorial em disco (partições + KD-tree).

pub const MAGIC: [u8; 8] = *b"FRAUDIDX";
pub const VERSION: u32 = 4;

pub const VECTOR_DIM: usize = 14;
pub const PACKED_DIMS: usize = 16; // padding to 32-byte alignment for bbox arrays
pub const QUANT_SCALE: i32 = 10000;
pub const TOP_K: usize = 5;
pub const LANES: usize = 8;

pub const HEADER_SIZE: usize = 64;
pub const PART_SIZE: usize = 76; // 4+4+4+32+32
pub const NODE_SIZE: usize = 80; // 4+4+4+4+32+32
pub const BLOCK_BYTES: usize = VECTOR_DIM * LANES * 2; // 14*8*i16 = 224
/// MCC risk lookup table size (mcc % MCC_TABLE_SIZE → i16 risk).
pub const MCC_TABLE_SIZE: usize = 1024;

pub const EARLY_DISTANCE_MILLI: i32 = 140;
/// In i16-quantized space: ((SCALE * 140) / 1000)^2 = 1400^2 = 1_960_000.
/// Squared L2 below this and we stop the whole search — top-5 are "very close".
pub const EARLY_DISTANCE_LIMIT: i64 = {
    let v = (QUANT_SCALE * EARLY_DISTANCE_MILLI / 1000) as i64;
    v * v
};

pub type QueryVector = [i16; PACKED_DIMS];

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IndexHeader {
    pub magic: [u8; 8],
    pub scale: u32,
    pub dims: u32,
    pub packed_dims: u32,
    pub lanes: u32,
    pub ref_count: u32,
    pub part_count: u32,
    pub node_count: u32,
    pub block_count: u32,
    /// Byte offset of the MCC risk table (MCC_TABLE_SIZE × i16).
    pub mcc_table_offset: u32,
    pub _padding: [u8; 20],
}

const _: () = {
    assert!(std::mem::size_of::<IndexHeader>() == HEADER_SIZE);
};

#[cfg(test)]
mod header_tests {
    use super::*;
    #[test]
    fn header_size_64() {
        assert_eq!(std::mem::size_of::<BlobHeader>(), 64);
    }
    #[test]
    fn magic_value() {
        assert_eq!(MAGIC, *b"FRAUDIDX");
    }
}

// ---------------------------------------------------------------------------
// Quantization helpers — thresholds must match between index build and search.
// ---------------------------------------------------------------------------

/// `value` is expected to be in `[-1.0, 1.0]`. Maps to i16 scaled by QUANT_SCALE.
/// Sentinel of `-1.0` round-trips to `-QUANT_SCALE`.
#[inline]
pub fn quantize_value(value: f64) -> i16 {
    if value <= -1.0 {
        return -(QUANT_SCALE as i16);
    }
    if value <= 0.0 {
        return 0;
    }
    if value >= 1.0 {
        return QUANT_SCALE as i16;
    }
    let scaled = (value * QUANT_SCALE as f64).round();
    scaled as i16
}

#[inline]
fn clamp_quant_u64(v: u64) -> i16 {
    if v >= QUANT_SCALE as u64 {
        QUANT_SCALE as i16
    } else {
        v as i16
    }
}

#[inline]
fn div_round_u64(num: u64, denom: u64) -> u64 {
    (num + denom / 2) / denom
}

/// Map a non-negative integer in `[0, denominator]` to i16 scaled by QUANT_SCALE.
#[inline]
pub fn quantize_uint_div(value: u32, denominator: u32) -> i16 {
    clamp_quant_u64(div_round_u64(
        value as u64 * QUANT_SCALE as u64,
        denominator as u64,
    ))
}

/// Map a millis value (value * 1000) divided by `denominator_units` to i16.
#[inline]
pub fn quantize_milli_div(value_milli: u32, denominator_units: u32) -> i16 {
    clamp_quant_u64(div_round_u64(
        value_milli as u64 * QUANT_SCALE as u64,
        denominator_units as u64 * 1000,
    ))
}

/// `amount / avg`, clamped to `[0, SCALE]`. Matches the C ratio formula:
/// `(amount_milli * 1000) / avg_milli`.
#[inline]
pub fn quantize_amount_ratio(amount_milli: u32, avg_milli: u32) -> i16 {
    if avg_milli == 0 {
        return QUANT_SCALE as i16;
    }
    clamp_quant_u64(div_round_u64(amount_milli as u64 * 1000, avg_milli as u64))
}

// ---------------------------------------------------------------------------
// Feature input / vectorization
// ---------------------------------------------------------------------------

pub const MAX_AMOUNT_UNITS: u32 = 10_000;
pub const MAX_INSTALLMENTS: u32 = 12;
pub const MAX_HOUR: u32 = 23;
pub const MAX_DOW: u32 = 6;
pub const MAX_MINUTES: u32 = 1440;
pub const MAX_KM_UNITS: u32 = 1000;
pub const MAX_TX_COUNT_24H: u32 = 20;
pub const MAX_MERCHANT_AVG_UNITS: u32 = 10_000;

/// Parsed payload features in raw quantization-ready units. `milli` fields are
/// value × 1000 (so floats with up to 3 decimals are represented exactly).
#[derive(Debug, Clone, Copy)]
pub struct RawFeatures {
    pub amount_milli: u32,
    pub installments: u32,
    pub hour_of_day: u8,
    pub day_of_week: u8,
    pub minutes_since_last_tx: Option<u32>,
    pub km_from_last_tx_milli: Option<u32>,
    pub km_from_home_milli: u32,
    pub customer_avg_amount_milli: u32,
    pub tx_count_24h: u32,
    pub is_online: bool,
    pub card_present: bool,
    pub unknown_merchant: bool,
    /// Pre-quantized mcc_risk in i16 space `[0, QUANT_SCALE]` — looked up from
    /// the per-MCC table in the builder, hardcoded values at the API.
    pub mcc_risk_q: i16,
    pub merchant_avg_amount_milli: u32,
}

/// Produce a 16-dim i16 vector. Dims 0..14 are real features; 14..16 are 0
/// padding so the `[i16; 16]` arrays align cleanly for AVX2 loads on bboxes.
#[inline(always)]
pub fn vectorize(r: &RawFeatures) -> QueryVector {
    let mut v: QueryVector = [0; PACKED_DIMS];
    v[0] = quantize_milli_div(r.amount_milli, MAX_AMOUNT_UNITS);
    v[1] = quantize_uint_div(r.installments, MAX_INSTALLMENTS);
    v[2] = quantize_amount_ratio(r.amount_milli, r.customer_avg_amount_milli);
    v[3] = quantize_uint_div(r.hour_of_day as u32, MAX_HOUR);
    v[4] = quantize_uint_div(r.day_of_week as u32, MAX_DOW);
    match r.minutes_since_last_tx {
        Some(m) => v[5] = quantize_uint_div(m, MAX_MINUTES),
        None => v[5] = -(QUANT_SCALE as i16),
    }
    match r.km_from_last_tx_milli {
        Some(km) => v[6] = quantize_milli_div(km, MAX_KM_UNITS),
        None => v[6] = -(QUANT_SCALE as i16),
    }
    v[7] = quantize_milli_div(r.km_from_home_milli, MAX_KM_UNITS);
    v[8] = quantize_uint_div(r.tx_count_24h, MAX_TX_COUNT_24H);
    v[9] = if r.is_online { QUANT_SCALE as i16 } else { 0 };
    v[10] = if r.card_present {
        QUANT_SCALE as i16
    } else {
        0
    };
    v[11] = if r.unknown_merchant {
        QUANT_SCALE as i16
    } else {
        0
    };
    v[12] = r.mcc_risk_q;
    v[13] = quantize_milli_div(r.merchant_avg_amount_milli, MAX_MERCHANT_AVG_UNITS);
    v
}

// ---------------------------------------------------------------------------
// Partitioning + lower-bound distance for KD-tree pruning
// ---------------------------------------------------------------------------

/// Build a discrete partition key from query features.
/// `partition_key` exactly so a query and its true neighbors fall in the
/// same partition with high probability.
#[inline]
pub fn partition_key(v: &QueryVector) -> u32 {
    let mut key: u32 = 0;
    if v[5] >= 0 {
        key |= 1 << 0;
    }
    if v[9] > 0 {
        key |= 1 << 1;
    }
    if v[10] > 0 {
        key |= 1 << 2;
    }
    if v[11] > 0 {
        key |= 1 << 3;
    }
    let mr = v[12];
    if mr <= 2047 {
        // bucket 0
    } else if mr <= 4095 {
        key |= 1 << 4;
    } else if mr <= 6143 {
        key |= 2 << 4;
    } else {
        key |= 3 << 4;
    }
    if v[2] > 4096 {
        key |= 1 << 6;
    }
    if v[8] > 2048 {
        key |= 1 << 7;
    }
    key
}

#[inline]
fn lower_bound_dim(q: i16, lo: i16, hi: i16) -> i64 {
    let diff: i64 = if q < lo {
        lo as i64 - q as i64
    } else if q > hi {
        q as i64 - hi as i64
    } else {
        0
    };
    diff * diff
}

/// Like `lower_bound_vec`, but stops early once the partial sum reaches `cutoff`.
#[inline(always)]
pub fn lower_bound_vec_cutoff(
    q: &QueryVector,
    min: &QueryVector,
    max: &QueryVector,
    cutoff: i64,
) -> i64 {
    let mut acc = 0i64;
    macro_rules! step {
        ($d:expr) => {{
            acc += lower_bound_dim(q[$d], min[$d], max[$d]);
            if acc >= cutoff {
                return acc;
            }
        }};
    }
    step!(0);
    step!(1);
    step!(2);
    step!(3);
    step!(4);
    step!(5);
    step!(6);
    step!(7);
    step!(8);
    step!(9);
    step!(10);
    step!(11);
    step!(12);
    step!(13);
    acc
}

/// Sum of per-dim lower-bound contributions. Lower bound on squared L2 to
/// any vector inside the axis-aligned box `[min, max]`. Cheap and correct.
#[inline(always)]
pub fn lower_bound_vec(q: &QueryVector, min: &QueryVector, max: &QueryVector) -> i64 {
    lower_bound_dim(q[0], min[0], max[0])
        + lower_bound_dim(q[1], min[1], max[1])
        + lower_bound_dim(q[2], min[2], max[2])
        + lower_bound_dim(q[3], min[3], max[3])
        + lower_bound_dim(q[4], min[4], max[4])
        + lower_bound_dim(q[5], min[5], max[5])
        + lower_bound_dim(q[6], min[6], max[6])
        + lower_bound_dim(q[7], min[7], max[7])
        + lower_bound_dim(q[8], min[8], max[8])
        + lower_bound_dim(q[9], min[9], max[9])
        + lower_bound_dim(q[10], min[10], max[10])
        + lower_bound_dim(q[11], min[11], max[11])
        + lower_bound_dim(q[12], min[12], max[12])
        + lower_bound_dim(q[13], min[13], max[13])
}

#[cfg(test)]
mod helpers_tests {
    use super::*;

    #[test]
    fn quantize_unit_endpoints() {
        assert_eq!(quantize_value(-1.0), -10000);
        assert_eq!(quantize_value(0.0), 0);
        assert_eq!(quantize_value(0.5), 5000);
        assert_eq!(quantize_value(1.0), 10000);
    }

    #[test]
    fn partition_key_obvious() {
        let mut v: QueryVector = [0; PACKED_DIMS];
        // all-zero → key = 1 (bit 0 set because v[5]==0 >= 0)
        assert_eq!(partition_key(&v), 0b00000001);
        v[5] = -10000;
        assert_eq!(partition_key(&v), 0); // sentinel last-tx clears bit 0
    }

    #[test]
    fn lower_bound_zero_inside_box() {
        let q: QueryVector = [100; PACKED_DIMS];
        let lo: QueryVector = [50; PACKED_DIMS];
        let hi: QueryVector = [200; PACKED_DIMS];
        assert_eq!(lower_bound_vec(&q, &lo, &hi), 0);
    }

    #[test]
    fn lower_bound_outside() {
        let q: QueryVector = [300; PACKED_DIMS]; // beyond hi=200
        let lo: QueryVector = [50; PACKED_DIMS];
        let hi: QueryVector = [200; PACKED_DIMS];
        // diff per dim = 100, squared = 10000, over 14 dims = 140000
        assert_eq!(lower_bound_vec(&q, &lo, &hi), 14 * 10000);
        assert_eq!(
            lower_bound_vec_cutoff(&q, &lo, &hi, i64::MAX),
            14 * 10000
        );
        assert!(lower_bound_vec_cutoff(&q, &lo, &hi, 50000) >= 50000);
    }
}
