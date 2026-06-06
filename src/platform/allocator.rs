//! Runtime heap tuning for Linux binaries.

#[inline(always)]
pub fn set_malloc_tuning() {
    std::env::set_var("MALLOC_ARENA_MAX", "1");
    std::env::set_var("MALLOC_TRIM_THRESHOLD", "131072");
    std::env::set_var("MALLOC_MMAP_THRESHOLD", "131072");

    unsafe {
        extern "C" {
            fn mallopt(param: libc::c_int, value: libc::c_int) -> libc::c_int;
        }
        const M_TRIM_THRESHOLD: libc::c_int = -1;
        const M_MMAP_THRESHOLD: libc::c_int = -3;
        const M_ARENA_MAX: libc::c_int = -8;
        let _ = mallopt(M_ARENA_MAX, 1);
        let _ = mallopt(M_TRIM_THRESHOLD, 131072);
        let _ = mallopt(M_MMAP_THRESHOLD, 131072);
    }
}
