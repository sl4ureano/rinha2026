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
    fraud_detector::platform::allocator::set_malloc_tuning();
    let cfg = fraud_detector::config::LbConfig::fixed();
    fraud_detector::perf::init_from_env();
    fraud_detector::platform::load_balancer::run(cfg);
}
