//! Request parsing (hot path) + vectorização opcional (k-NN).

#[cfg(feature = "knn-index")]
mod features;

mod cache;
mod json;
mod linear;
mod numbers;

#[cfg(feature = "knn-index")]
pub use features::vectorize_payload;

pub use cache::TierCache;
pub use json::{extract, extract_filled, RawPayload};
pub use cache::{fill, fill_base, fill_datetime};
