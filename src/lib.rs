//! Fraud detection API for Rinha 2026 — tier scorer + optional k-NN index tooling.

pub mod config;
pub mod index;
pub mod search;
pub mod ingest;
pub mod http;
pub mod platform;

pub mod build;

pub use config::ServerConfig;
pub use index::Index;
