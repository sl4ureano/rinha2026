//! HTTP serving on monoio (pipelined requests, static response bodies).

mod handler;
pub mod response;
pub mod runtime;

#[cfg(target_os = "linux")]
mod sync_handler;

pub use handler::handle_connection;

#[cfg(target_os = "linux")]
pub use sync_handler::serve_connection;
