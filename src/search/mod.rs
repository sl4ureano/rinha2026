//! Classificação: gasto seguro no fast path; demais casos no k-NN exato.

#[cfg(feature = "knn-index")]
mod fast_path;

#[cfg(feature = "knn-index")]
mod knn;

#[cfg(feature = "knn-index")]
pub use fast_path::try_fast_fraud_count;

#[cfg(feature = "knn-index")]
pub use knn::fraud_count;

#[cfg(feature = "knn-index")]
mod warmup;

#[cfg(feature = "knn-index")]
pub use warmup::run_warmup;
