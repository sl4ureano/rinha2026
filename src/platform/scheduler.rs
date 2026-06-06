//! Real-time scheduler setup for Linux.

#[inline(always)]
pub fn set_realtime_priority(priority: i32) {
    unsafe {
        let param = libc::sched_param {
            sched_priority: priority,
        };
        let ret = libc::sched_setscheduler(0, libc::SCHED_FIFO, &param);
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            eprintln!("sched_setscheduler(SCHED_FIFO, {}) failed: {} (need CAP_SYS_NICE)", 
                priority, err);
        } else {
            eprintln!("SCHED_FIFO activated (priority={})", priority);
        }
    }
}
