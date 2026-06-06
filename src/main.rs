use fraud_detector::config::ServerConfig;

#[cfg(all(target_os = "linux", not(debug_assertions)))]
#[global_allocator]
static GLOBAL: fraud_detector::perf::CountingAllocator<mimalloc::MiMalloc> =
    fraud_detector::perf::CountingAllocator(mimalloc::MiMalloc);

fn main() {
    fraud_detector::platform::allocator::set_malloc_tuning();
    let cfg = ServerConfig::from_args();
    fraud_detector::perf::init_from_env();

    #[cfg(target_os = "linux")]
    fraud_detector::platform::scheduler::set_realtime_priority(80);

    #[cfg(target_os = "linux")]
    {
        let port = cfg.health_port;
        let index = load_index(&cfg);
        fraud_detector::platform::fd_gateway::run(cfg.ctrl_sock.as_path(), index, port)
            .unwrap_or_else(|e| panic!("fd_gateway: {e}"));
    }

    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("server: Linux only");
        std::process::exit(1);
    }
}

#[cfg(feature = "knn-index")]
fn load_index(cfg: &ServerConfig) -> std::sync::Arc<fraud_detector::Index> {
    use std::sync::Arc;

    use fraud_detector::Index;

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
