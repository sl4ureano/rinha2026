use fraud_detector::config::ServerConfig;

#[cfg(all(target_os = "linux", not(debug_assertions)))]
#[global_allocator]
static GLOBAL: fraud_detector::perf::CountingAllocator<mimalloc::MiMalloc> =
    fraud_detector::perf::CountingAllocator(mimalloc::MiMalloc);

fn main() {
    let cfg = ServerConfig::from_env();
    fraud_detector::perf::init_from_env();

    #[cfg(target_os = "linux")]
    {
        if let Some(ctrl_sock) = &cfg.ctrl_sock {
            let port = cfg.health_port;
            let index = load_index(&cfg);
            fraud_detector::platform::fd_gateway::run(ctrl_sock.as_path(), index, port)
                .unwrap_or_else(|e| panic!("fd_gateway: {e}"));
            return;
        }

        if let Ok(port_str) = std::env::var("DIRECT_PORT") {
            let port: u16 = port_str.parse().expect("DIRECT_PORT must be a u16");
            let index = load_index(&cfg);
            fraud_detector::platform::fd_gateway::run_direct(index, port)
                .unwrap_or_else(|e| panic!("direct_tcp: {e}"));
            return;
        }
    }

    #[cfg(feature = "monoio-http")]
    {
        use std::sync::Arc;

        use fraud_detector::http::runtime;
        use fraud_detector::index::Index;

        let index = load_index(&cfg);
        let bind_addr = cfg
            .bind_addr
            .expect("BIND or PORT required when FD pass-through is disabled");
        runtime::run_tcp(index, bind_addr);
    }

    #[cfg(not(feature = "monoio-http"))]
    {
        eprintln!(
            "server: set FD_PASS/CTRL_SOCK (submission) or build with --features monoio-http"
        );
        std::process::exit(1);
    }
}

#[cfg(feature = "knn-index")]
fn load_index(cfg: &ServerConfig) -> std::sync::Arc<fraud_detector::Index> {
    use std::sync::Arc;

    use fraud_detector::Index;

    fn tier_only_mode() -> bool {
        fn env_truthy(key: &str) -> bool {
            std::env::var(key)
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        }
        env_truthy("TIER_ONLY") || env_truthy("SKIP_INDEX")
    }

    if tier_only_mode() {
        eprintln!("tier-only: index loading skipped");
        return Arc::new(Index::empty());
    }

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
}
