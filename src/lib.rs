//! Fraud detection API for Rinha 2026 — tier scorer + optional k-NN index tooling.

pub mod config;
pub mod http;
pub mod ingest;
pub mod perf;
pub mod platform;
pub mod search;

#[cfg(feature = "knn-index")]
pub mod index;

#[cfg(feature = "knn-index")]
pub mod build;

pub use config::ServerConfig;

#[cfg(feature = "knn-index")]
pub use index::Index;
