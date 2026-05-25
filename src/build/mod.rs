//! Offline index construction (used by the `build-index` binary).

mod entry;
mod pack;
mod sources;

pub use pack::{build_index, build_index_with_leaf, BuildInputs, DEFAULT_LEAF_SIZE};

pub mod cli;
