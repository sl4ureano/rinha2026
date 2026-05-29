//! Request parsing (hot path) + vectorização opcional (k-NN).

#[cfg(feature = "knn-index")]
mod features;

mod json;

#[cfg(feature = "knn-index")]
pub use features::vectorize_payload;

pub use json::{extract, RawPayload};
