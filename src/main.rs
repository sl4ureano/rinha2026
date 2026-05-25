use std::sync::Arc;

use fraud_detector::config::ServerConfig;
use fraud_detector::http::runtime;
use fraud_detector::index::Index;

#[cfg(all(target_os = "linux", not(debug_assertions)))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let cfg = ServerConfig::from_env();
    let index = Arc::new(
        Index::open(cfg.index_path())
            .unwrap_or_else(|e| panic!("index open {}: {e}", cfg.index_path().display())),
    );
    eprintln!(
        "index: {} partitions, {} nodes, {} blocks",
        index.part_count(),
        index.node_count(),
        index.block_count(),
    );

    #[cfg(target_os = "linux")]
    if let Some(ctrl_sock) = &cfg.ctrl_sock {
        let port = cfg.health_port;
        std::thread::spawn(move || runtime::health_tcp_loop(port));
        fraud_detector::platform::fd_gateway::run(index, ctrl_sock.as_path())
            .unwrap_or_else(|e| panic!("fd_gateway: {e}"));
        return;
    }

    let bind_addr = cfg
        .bind_addr
        .expect("BIND or PORT required when FD pass-through is disabled");
    runtime::run_tcp(index, bind_addr);
}
