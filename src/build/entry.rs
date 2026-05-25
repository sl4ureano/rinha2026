use crate::build::sources::ReferenceEntry;
use crate::index::{quantize_value, QueryVector, PACKED_DIMS, VECTOR_DIM};

/// Quantize a reference entry's pre-computed float vector to i16 with our SCALE.
/// Dims 14..16 are left at zero — only the real DIMS are touched.
pub fn entry_to_vector(e: &ReferenceEntry) -> (QueryVector, u8) {
    debug_assert!(
        e.vector.len() >= VECTOR_DIM,
        "reference vector too short: {}",
        e.vector.len()
    );
    let mut out: QueryVector = [0; PACKED_DIMS];
    for (i, &x) in e.vector.iter().enumerate().take(VECTOR_DIM) {
        out[i] = quantize_value(x as f64);
    }
    let fraud = if e.label == "fraud" { 1 } else { 0 };
    (out, fraud)
}
