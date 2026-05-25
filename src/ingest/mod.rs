//! Request parsing and feature extraction.

mod features;
mod json;

pub use features::vectorize_payload;
pub use json::{extract, RawPayload};
