//! HTTP: respostas estáticas (hot path) + health TCP; monoio só com feature `monoio-http`.

pub mod health;
pub mod response;

#[cfg(feature = "monoio-http")]
mod handler;

#[cfg(feature = "monoio-http")]
pub mod runtime;

#[cfg(all(target_os = "linux", feature = "monoio-http"))]
mod sync_handler;

#[cfg(feature = "monoio-http")]
pub use handler::handle_connection;

#[cfg(all(target_os = "linux", feature = "monoio-http"))]
pub use sync_handler::serve_connection;
