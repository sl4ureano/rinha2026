//! On-disk index layout, quantization, and memory-mapped access.

mod mmap;
mod quantize;

pub use mmap::Index;
pub use quantize::*;
