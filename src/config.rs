//! Runtime configuration from environment variables.

use std::path::{Path, PathBuf};

#[inline]
fn env_trim(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string())
}

#[inline]
fn env_trim_or(key: &str, default: &str) -> String {
    env_trim(key).unwrap_or_else(|| default.to_string())
}

/// API process configuration (index path, listen mode, health port).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub index_path: PathBuf,
    /// When set, the API receives accepted TCP sockets from the LB via Unix SCM_RIGHTS.
    pub ctrl_sock: Option<PathBuf>,
    pub health_port: u16,
    /// Direct TCP bind address when not using FD pass-through.
    pub bind_addr: Option<std::net::SocketAddr>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let index_path = env_trim("INDEX_PATH")
            .or_else(|| env_trim("BLOB_PATH"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/app/data/index.bin"));

        let ctrl_sock = env_trim("CTRL_SOCK")
            .map(PathBuf::from)
            .or_else(|| env_trim("RINHA_FD_SOCK").map(PathBuf::from))
            .or_else(|| {
                let fd_pass = env_trim("FD_PASS")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if fd_pass {
                    env_trim("SOCKET_PATH").map(PathBuf::from)
                } else {
                    None
                }
            });

        let health_port = env_trim_or("PORT", "8080")
            .parse()
            .unwrap_or(8080);

        let bind_addr = if ctrl_sock.is_some() {
            None
        } else {
            let bind_str = env_trim("BIND").unwrap_or_else(|| format!("0.0.0.0:{health_port}"));
            Some(bind_str.parse().expect("BIND must be a valid socket address"))
        };

        Self {
            index_path,
            ctrl_sock,
            health_port,
            bind_addr,
        }
    }

    pub fn index_path(&self) -> &Path {
        &self.index_path
    }
}

/// Load-balancer configuration.
#[derive(Debug, Clone)]
pub struct LbConfig {
    pub port: u16,
    pub api1_socket: String,
    pub api2_socket: String,
}

impl LbConfig {
    pub fn from_env() -> Self {
        Self {
            port: env_trim_or("LB_PORT", "9999")
                .parse()
                .expect("LB_PORT"),
            api1_socket: env_trim_or("API1_SOCKET", "/tmp/sockets/api1.sock"),
            api2_socket: env_trim_or("API2_SOCKET", "/tmp/sockets/api2.sock"),
        }
    }
}
