#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("lb: Linux only");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    let cfg = fraud_detector::config::LbConfig::from_env();
    fraud_detector::platform::load_balancer::run(cfg);
}
