//! Classificação: tier_score (submissão) + k-NN opcional (ferramentas).

mod decision_tree;
mod tier_score;

#[cfg(feature = "knn-index")]
mod fast_path;

#[cfg(feature = "knn-index")]
mod knn;

pub use tier_score::{
    complete_cache, ratio_only_count, tier_fraud_count, tier_gray_count, tier_path, tree_only_count,
    TierPath,
};

#[cfg(feature = "knn-index")]
pub use fast_path::try_fast_fraud_count;

#[cfg(feature = "knn-index")]
pub use knn::fraud_count;

#[cfg(feature = "knn-index")]
mod warmup;

#[cfg(feature = "knn-index")]
pub use warmup::run_warmup;
