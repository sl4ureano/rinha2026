//! Runtime configuration for the submission layout.

use std::path::{Path, PathBuf};

const INDEX_PATH: &str = "/app/data/index.bin";
const HEALTH_PORT: u16 = 8080;
const LB_PORT: u16 = 9999;
const API1_SOCKET: &str = "/tmp/sockets/api1.sock";
const API2_SOCKET: &str = "/tmp/sockets/api2.sock";
const CHANNELS_PER_API: usize = 2;

/// API process configuration (index path, listen mode, health port).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub index_path: PathBuf,
    pub ctrl_sock: PathBuf,
    pub health_port: u16,
}

impl ServerConfig {
    pub fn from_args() -> Self {
        let ctrl_sock = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .expect("server requires the control socket path as the first argument");

        Self {
            index_path: PathBuf::from(INDEX_PATH),
            ctrl_sock,
            health_port: HEALTH_PORT,
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
    pub upstreams: Vec<String>,
}

impl LbConfig {
    pub fn fixed() -> Self {
        let mut upstreams = Vec::with_capacity(CHANNELS_PER_API * 2);
        for _ in 0..CHANNELS_PER_API {
            upstreams.push(API1_SOCKET.to_string());
            upstreams.push(API2_SOCKET.to_string());
        }

        Self {
            port: LB_PORT,
            upstreams,
        }
    }
}
