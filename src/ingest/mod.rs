//! Request parsing (hot path) + vectorização opcional (k-NN).

#[cfg(feature = "knn-index")]
mod features;

mod cache;
mod json;

#[cfg(feature = "knn-index")]
pub use features::vectorize_payload;

pub use cache::TierCache;
pub use json::{extract, RawPayload};
