#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("lb: Linux only");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: fraud_detector::perf::CountingAllocator<std::alloc::System> =
    fraud_detector::perf::CountingAllocator(std::alloc::System);

#[cfg(target_os = "linux")]
fn main() {
    let cfg = fraud_detector::config::LbConfig::from_env();
    fraud_detector::perf::init_from_env();
    fraud_detector::platform::load_balancer::run(cfg);
}
