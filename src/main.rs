use std::sync::Arc;

use fraud_detector::config::ServerConfig;
use fraud_detector::http::runtime;
use fraud_detector::index::Index;

#[cfg(all(target_os = "linux", not(debug_assertions)))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn tier_only_mode() -> bool {
    fn env_truthy(key: &str) -> bool {
        std::env::var(key)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }
    env_truthy("TIER_ONLY") || env_truthy("SKIP_INDEX") || env_truthy("FD_PASS")
}

fn main() {
    let cfg = ServerConfig::from_env();
    let index = if tier_only_mode() {
        eprintln!("tier-only: index loading skipped (FD_PASS mode)");
        Arc::new(Index::empty())
    } else {
        let idx = Arc::new(
            Index::open(cfg.index_path())
                .unwrap_or_else(|e| panic!("index open {}: {e}", cfg.index_path().display())),
        );
        eprintln!(
            "index: {} partitions, {} nodes, {} blocks",
            idx.part_count(),
            idx.node_count(),
            idx.block_count(),
        );
        idx
    };

    #[cfg(target_os = "linux")]
    {
        // Direct TCP mode: bypass LB, listen on port directly with raw epoll
        if let Ok(port_str) = std::env::var("DIRECT_PORT") {
            let port: u16 = port_str.parse().expect("DIRECT_PORT must be a u16");
            fraud_detector::platform::fd_gateway::run_direct(index, port)
                .unwrap_or_else(|e| panic!("direct_tcp: {e}"));
            return;
        }

        if let Some(ctrl_sock) = &cfg.ctrl_sock {
            let port = cfg.health_port;
            std::thread::spawn(move || runtime::health_tcp_loop(port));
            fraud_detector::platform::fd_gateway::run(index, ctrl_sock.as_path())
                .unwrap_or_else(|e| panic!("fd_gateway: {e}"));
            return;
        }
    }

    let bind_addr = cfg
        .bind_addr
        .expect("BIND or PORT required when FD pass-through is disabled");
    runtime::run_tcp(index, bind_addr);
}
