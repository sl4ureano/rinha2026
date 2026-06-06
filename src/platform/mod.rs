//! Linux-specific runtime integration (FD pass-through, load balancer).

#[cfg(target_os = "linux")]
pub mod allocator;

#[cfg(target_os = "linux")]
pub mod fd_gateway;

#[cfg(target_os = "linux")]
pub mod load_balancer;

#[cfg(target_os = "linux")]
pub mod scheduler;

#[cfg(target_os = "linux")]
pub mod scm;
