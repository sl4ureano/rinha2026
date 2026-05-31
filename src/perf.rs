use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub const STAGE_LB_ACCEPT: usize = 0;
pub const STAGE_LB_HANDOFF: usize = 1;
pub const STAGE_API_RECV_FD: usize = 2;
pub const STAGE_API_SETSOCKOPT: usize = 3;
pub const STAGE_SPIN_READ: usize = 4;
pub const STAGE_SOCKET_RECV: usize = 5;
pub const STAGE_HTTP_PARSE: usize = 6;
pub const STAGE_VALIDATION: usize = 7;
pub const STAGE_PARSE_JSON: usize = 8;
pub const STAGE_PAYLOAD_CACHE_FILL: usize = 9;
pub const STAGE_FAST_PATH_LOOKUP: usize = 10;
pub const STAGE_DECISION_TREE: usize = 11;
pub const STAGE_RESPONSE_SELECT: usize = 12;
pub const STAGE_SEND_SYSCALL: usize = 13;
pub const STAGE_WRITE_COMPLETE: usize = 14;
pub const STAGE_SERVER_PROCESSING: usize = 15;
pub const STAGE_REQUEST_TOTAL: usize = 16;
pub const STAGE_EPOLL_WAIT: usize = 17;
pub const STAGE_EPOLL_DISPATCH: usize = 18;

// Backward-compatible names used by existing call sites.
pub const STAGE_CACHE_LOOKUP: usize = STAGE_FAST_PATH_LOOKUP;
pub const STAGE_CACHE_INSERT: usize = STAGE_PAYLOAD_CACHE_FILL;
pub const STAGE_SERIALIZE: usize = STAGE_RESPONSE_SELECT;
pub const STAGE_WRITE_RESPONSE: usize = STAGE_SEND_SYSCALL;

const STAGE_COUNT: usize = 19;
const HIST_BUCKET_COUNT: usize = 42;
const HIST_BUCKETS_US: [u64; HIST_BUCKET_COUNT] = [
    1,
    2,
    3,
    4,
    5,
    7,
    10,
    15,
    20,
    25,
    30,
    40,
    50,
    60,
    75,
    100,
    125,
    150,
    200,
    250,
    300,
    400,
    500,
    600,
    750,
    1_000,
    1_250,
    1_500,
    2_000,
    3_000,
    5_000,
    7_500,
    10_000,
    15_000,
    20_000,
    30_000,
    50_000,
    75_000,
    100_000,
    250_000,
    1_000_000,
    u64::MAX,
];
const STAGE_NAMES: [&str; STAGE_COUNT] = [
    "lb_accept",
    "lb_handoff_scm_rights",
    "api_recv_fd",
    "api_setsockopt",
    "spin_read",
    "socket_recv",
    "http_parse",
    "validation",
    "parse_json",
    "payload_cache_fill",
    "fast_path_lookup",
    "decision_tree",
    "response_select_static",
    "send_syscall",
    "write_complete",
    "server_processing",
    "request_total",
    "epoll_wait",
    "epoll_dispatch",
];

static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOC_PROFILING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

static REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);
static REQUESTS_SUCCESS: AtomicU64 = AtomicU64::new(0);
static REQUESTS_ERROR: AtomicU64 = AtomicU64::new(0);
static ACTIVE_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static BYTES_RECEIVED: AtomicU64 = AtomicU64::new(0);
static BYTES_SENT: AtomicU64 = AtomicU64::new(0);
static WEBHOOK_QUEUE_LEN: AtomicU64 = AtomicU64::new(0);
static WEBHOOK_DROPPED: AtomicU64 = AtomicU64::new(0);
static WEBHOOK_SEND_OK: AtomicU64 = AtomicU64::new(0);
static WEBHOOK_SEND_ERR: AtomicU64 = AtomicU64::new(0);

static LB_ACCEPTED: AtomicU64 = AtomicU64::new(0);
static LB_HANDOFF_OK: AtomicU64 = AtomicU64::new(0);
static LB_HANDOFF_ERR: AtomicU64 = AtomicU64::new(0);
static LB_UPSTREAM_API1: AtomicU64 = AtomicU64::new(0);
static LB_UPSTREAM_API2: AtomicU64 = AtomicU64::new(0);

static RECV_FD_TOTAL: AtomicU64 = AtomicU64::new(0);
static SPIN_READ_HIT: AtomicU64 = AtomicU64::new(0);
static SPIN_READ_MISS: AtomicU64 = AtomicU64::new(0);
static SPIN_READ_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static EPOLL_READ_FALLBACK: AtomicU64 = AtomicU64::new(0);
static PARTIAL_WRITES: AtomicU64 = AtomicU64::new(0);
static WRITE_EAGAIN: AtomicU64 = AtomicU64::new(0);
static RECV_CALLS: AtomicU64 = AtomicU64::new(0);
static SEND_CALLS: AtomicU64 = AtomicU64::new(0);
static EPOLL_WAIT_CALLS: AtomicU64 = AtomicU64::new(0);
static EPOLL_WAIT_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static EPOLL_WAIT_ERRORS: AtomicU64 = AtomicU64::new(0);
static EPOLL_WAIT_EVENTS: AtomicU64 = AtomicU64::new(0);
static EPOLL_WAIT_MAX_EVENTS: AtomicU64 = AtomicU64::new(0);
static EPOLL_BUSY_POLL_SUPPORTED: AtomicBool = AtomicBool::new(false);
static EPOLL_BUSY_POLL_ENABLED: AtomicBool = AtomicBool::new(false);
static EPOLL_BUSY_POLL_USECS: AtomicU64 = AtomicU64::new(0);
static EPOLL_BUSY_POLL_BUDGET: AtomicU64 = AtomicU64::new(0);
static EPOLL_BUSY_POLL_PREFER: AtomicU64 = AtomicU64::new(0);
static EPOLL_BUSY_POLL_SET_ERRNO: AtomicU64 = AtomicU64::new(0);
static EPOLL_BUSY_POLL_GET_ERRNO: AtomicU64 = AtomicU64::new(0);

static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static DEALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static REALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static REALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

static STAGE_SUM_US: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];
static STAGE_COUNT_TOTAL: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];
static STAGE_MAX_US: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];
static STAGE_HIST: [[AtomicU64; HIST_BUCKET_COUNT]; STAGE_COUNT] =
    [const { [const { AtomicU64::new(0) }; HIST_BUCKET_COUNT] }; STAGE_COUNT];

static WEBHOOK_TX: OnceLock<SyncSender<String>> = OnceLock::new();
static WEBHOOK_URL: OnceLock<String> = OnceLock::new();
static RELAY_INBOX: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
static RELAY_TARGET: OnceLock<String> = OnceLock::new();

pub struct CountingAllocator<A>(pub A);

unsafe impl<A: GlobalAlloc> GlobalAlloc for CountingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = self.0.alloc(layout);
        if ALLOC_PROFILING.load(Ordering::Relaxed) {
            ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ALLOC_PROFILING.load(Ordering::Relaxed) {
            DEALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        }
        self.0.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let next = self.0.realloc(ptr, layout, new_size);
        if ALLOC_PROFILING.load(Ordering::Relaxed) {
            REALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            REALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        next
    }
}

#[derive(Clone)]
struct SnapshotState {
    last_instant: Instant,
    last_cpu: Option<CpuTicks>,
    last_rss_kb: u64,
    prev_requests: u64,
    prev_success: u64,
    prev_error: u64,
    prev_stage_counts: [u64; STAGE_COUNT],
    prev_stage_sums: [u64; STAGE_COUNT],
    prev_stage_hist: [[u64; HIST_BUCKET_COUNT]; STAGE_COUNT],
    prev_alloc_calls: u64,
    prev_alloc_bytes: u64,
    prev_realloc_calls: u64,
    prev_realloc_bytes: u64,
    prev_epoll_wait_calls: u64,
    prev_epoll_wait_timeouts: u64,
    prev_epoll_wait_errors: u64,
    prev_epoll_wait_events: u64,
}

impl Default for SnapshotState {
    fn default() -> Self {
        Self {
            last_instant: Instant::now(),
            last_cpu: read_cpu_ticks(),
            last_rss_kb: read_proc_status().map(|m| m.rss_kb).unwrap_or(0),
            prev_requests: 0,
            prev_success: 0,
            prev_error: 0,
            prev_stage_counts: [0; STAGE_COUNT],
            prev_stage_sums: [0; STAGE_COUNT],
            prev_stage_hist: [[0; HIST_BUCKET_COUNT]; STAGE_COUNT],
            prev_alloc_calls: 0,
            prev_alloc_bytes: 0,
            prev_realloc_calls: 0,
            prev_realloc_bytes: 0,
            prev_epoll_wait_calls: 0,
            prev_epoll_wait_timeouts: 0,
            prev_epoll_wait_errors: 0,
            prev_epoll_wait_events: 0,
        }
    }
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn init_from_env() {
    if !env_truthy("ENABLE_REMOTE_PROFILING") {
        return;
    }

    ENABLED.store(true, Ordering::Relaxed);
    ALLOC_PROFILING.store(
        env_truthy("ENABLE_REMOTE_PROFILING_ALLOC"),
        Ordering::Relaxed,
    );
    install_shutdown_signal_handlers();

    if let Ok(socket) = std::env::var("PERF_RELAY_SOCKET") {
        if !socket.trim().is_empty() {
            let _ = RELAY_TARGET.set(socket.clone());
            if env_truthy("PERF_RELAY_LISTEN") {
                start_relay_listener(socket);
            }
        }
    }

    if let Ok(webhook) = std::env::var("PERF_WEBHOOK_URL") {
        if !webhook.trim().is_empty() {
            start_webhook_sender(webhook);
        }
    }

    thread::Builder::new()
        .name("remote-perf-snapshot".to_string())
        .spawn(snapshot_loop)
        .ok();
}

pub fn reset() {
    REQUESTS_TOTAL.store(0, Ordering::Relaxed);
    REQUESTS_SUCCESS.store(0, Ordering::Relaxed);
    REQUESTS_ERROR.store(0, Ordering::Relaxed);
    ACTIVE_CONNECTIONS.store(0, Ordering::Relaxed);
    CACHE_HITS.store(0, Ordering::Relaxed);
    CACHE_MISSES.store(0, Ordering::Relaxed);
    BYTES_RECEIVED.store(0, Ordering::Relaxed);
    BYTES_SENT.store(0, Ordering::Relaxed);
    LB_ACCEPTED.store(0, Ordering::Relaxed);
    LB_HANDOFF_OK.store(0, Ordering::Relaxed);
    LB_HANDOFF_ERR.store(0, Ordering::Relaxed);
    LB_UPSTREAM_API1.store(0, Ordering::Relaxed);
    LB_UPSTREAM_API2.store(0, Ordering::Relaxed);
    RECV_FD_TOTAL.store(0, Ordering::Relaxed);
    SPIN_READ_HIT.store(0, Ordering::Relaxed);
    SPIN_READ_MISS.store(0, Ordering::Relaxed);
    SPIN_READ_ATTEMPTS.store(0, Ordering::Relaxed);
    EPOLL_READ_FALLBACK.store(0, Ordering::Relaxed);
    PARTIAL_WRITES.store(0, Ordering::Relaxed);
    WRITE_EAGAIN.store(0, Ordering::Relaxed);
    RECV_CALLS.store(0, Ordering::Relaxed);
    SEND_CALLS.store(0, Ordering::Relaxed);
    EPOLL_WAIT_CALLS.store(0, Ordering::Relaxed);
    EPOLL_WAIT_TIMEOUTS.store(0, Ordering::Relaxed);
    EPOLL_WAIT_ERRORS.store(0, Ordering::Relaxed);
    EPOLL_WAIT_EVENTS.store(0, Ordering::Relaxed);
    EPOLL_WAIT_MAX_EVENTS.store(0, Ordering::Relaxed);
    ALLOC_CALLS.store(0, Ordering::Relaxed);
    DEALLOC_CALLS.store(0, Ordering::Relaxed);
    REALLOC_CALLS.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    REALLOC_BYTES.store(0, Ordering::Relaxed);
    for stage in 0..STAGE_COUNT {
        STAGE_SUM_US[stage].store(0, Ordering::Relaxed);
        STAGE_COUNT_TOTAL[stage].store(0, Ordering::Relaxed);
        STAGE_MAX_US[stage].store(0, Ordering::Relaxed);
        for bucket in 0..HIST_BUCKET_COUNT {
            STAGE_HIST[stage][bucket].store(0, Ordering::Relaxed);
        }
    }
}

pub fn set_epoll_busy_poll_result(
    supported: bool,
    busy_poll_enabled: bool,
    usecs: u32,
    budget: u16,
    prefer: u8,
    set_errno: i32,
    get_errno: i32,
) {
    if !enabled() {
        return;
    }
    EPOLL_BUSY_POLL_SUPPORTED.store(supported, Ordering::Relaxed);
    EPOLL_BUSY_POLL_ENABLED.store(busy_poll_enabled, Ordering::Relaxed);
    EPOLL_BUSY_POLL_USECS.store(usecs as u64, Ordering::Relaxed);
    EPOLL_BUSY_POLL_BUDGET.store(budget as u64, Ordering::Relaxed);
    EPOLL_BUSY_POLL_PREFER.store(prefer as u64, Ordering::Relaxed);
    EPOLL_BUSY_POLL_SET_ERRNO.store(set_errno.max(0) as u64, Ordering::Relaxed);
    EPOLL_BUSY_POLL_GET_ERRNO.store(get_errno.max(0) as u64, Ordering::Relaxed);
}

#[inline]
pub fn record_epoll_wait(start: Option<Instant>, nfds: i32) {
    if !enabled() {
        return;
    }
    EPOLL_WAIT_CALLS.fetch_add(1, Ordering::Relaxed);
    match nfds {
        n if n > 0 => {
            let events = n as u64;
            EPOLL_WAIT_EVENTS.fetch_add(events, Ordering::Relaxed);
            update_max(&EPOLL_WAIT_MAX_EVENTS, events);
        }
        0 => {
            EPOLL_WAIT_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            EPOLL_WAIT_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
    }
    record_stage(STAGE_EPOLL_WAIT, start);
}

#[inline]
pub fn record_epoll_dispatch(start: Option<Instant>) {
    record_stage(STAGE_EPOLL_DISPATCH, start);
}

#[inline]
pub fn stage_start() -> Option<Instant> {
    if enabled() {
        Some(Instant::now())
    } else {
        None
    }
}

#[inline]
pub fn record_stage(stage: usize, start: Option<Instant>) {
    if let Some(start) = start {
        record_stage_us(stage, elapsed_us(start));
    }
}

#[inline]
pub fn record_stage_us(stage: usize, us: u64) {
    if !enabled() || stage >= STAGE_COUNT {
        return;
    }
    STAGE_SUM_US[stage].fetch_add(us, Ordering::Relaxed);
    STAGE_COUNT_TOTAL[stage].fetch_add(1, Ordering::Relaxed);
    update_max(&STAGE_MAX_US[stage], us);
    record_stage_hist(stage, us);
}

#[inline]
pub fn record_request(total_start: Option<Instant>, success: bool) {
    if let Some(start) = total_start {
        let us = elapsed_us(start);
        REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
        if success {
            REQUESTS_SUCCESS.fetch_add(1, Ordering::Relaxed);
        } else {
            REQUESTS_ERROR.fetch_add(1, Ordering::Relaxed);
        }
        record_stage_us(STAGE_REQUEST_TOTAL, us);
    }
}

#[inline]
pub fn record_request_us(us: u64, success: bool) {
    if !enabled() {
        return;
    }
    REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if success {
        REQUESTS_SUCCESS.fetch_add(1, Ordering::Relaxed);
    } else {
        REQUESTS_ERROR.fetch_add(1, Ordering::Relaxed);
    }
    record_stage_us(STAGE_REQUEST_TOTAL, us);
}

#[inline]
pub fn connection_opened() {
    if enabled() {
        ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn connection_closed() {
    if enabled() {
        ACTIVE_CONNECTIONS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1))
            .ok();
    }
}

pub struct ConnectionGuard;

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        connection_closed();
    }
}

pub fn connection_guard() -> Option<ConnectionGuard> {
    if enabled() {
        connection_opened();
        Some(ConnectionGuard)
    } else {
        None
    }
}

#[inline]
pub fn add_bytes_received(n: usize) {
    if enabled() {
        BYTES_RECEIVED.fetch_add(n as u64, Ordering::Relaxed);
    }
}

#[inline]
pub fn add_bytes_sent(n: usize) {
    if enabled() {
        BYTES_SENT.fetch_add(n as u64, Ordering::Relaxed);
    }
}

#[inline]
pub fn cache_hit() {
    if enabled() {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn cache_miss() {
    if enabled() {
        CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn lb_accepted() {
    if enabled() {
        LB_ACCEPTED.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn lb_handoff(ok: bool, upstream_idx: usize) {
    if !enabled() {
        return;
    }
    if ok {
        LB_HANDOFF_OK.fetch_add(1, Ordering::Relaxed);
        if upstream_idx == 0 {
            LB_UPSTREAM_API1.fetch_add(1, Ordering::Relaxed);
        } else {
            LB_UPSTREAM_API2.fetch_add(1, Ordering::Relaxed);
        }
    } else {
        LB_HANDOFF_ERR.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn recv_fd_ok() {
    if enabled() {
        RECV_FD_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn spin_read_result(hit: bool, attempts: u64) {
    if !enabled() {
        return;
    }
    SPIN_READ_ATTEMPTS.fetch_add(attempts, Ordering::Relaxed);
    if hit {
        SPIN_READ_HIT.fetch_add(1, Ordering::Relaxed);
    } else {
        SPIN_READ_MISS.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn epoll_read_fallback() {
    if enabled() {
        EPOLL_READ_FALLBACK.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn recv_call() {
    if enabled() {
        RECV_CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn send_call() {
    if enabled() {
        SEND_CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn partial_write() {
    if enabled() {
        PARTIAL_WRITES.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn write_eagain() {
    if enabled() {
        WRITE_EAGAIN.fetch_add(1, Ordering::Relaxed);
    }
}

fn start_webhook_sender(webhook: String) {
    let _ = WEBHOOK_URL.set(webhook.clone());
    let queue_cap = env_u64("PERF_WEBHOOK_QUEUE_CAP", 8) as usize;
    let (tx, rx) = sync_channel::<String>(queue_cap.max(1));
    let _ = WEBHOOK_TX.set(tx);

    thread::Builder::new()
        .name("remote-perf-webhook".to_string())
        .spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_millis(env_u64(
                    "PERF_WEBHOOK_TIMEOUT_MS",
                    800,
                )))
                .build();
            while let Ok(body) = rx.recv() {
                WEBHOOK_QUEUE_LEN.fetch_sub(1, Ordering::Relaxed);
                match agent
                    .post(&webhook)
                    .set("Content-Type", "application/json")
                    .send_string(&body)
                {
                    Ok(_) => {
                        WEBHOOK_SEND_OK.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        WEBHOOK_SEND_ERR.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        })
        .ok();
}

fn start_relay_listener(_socket: String) {
    let inbox = Arc::new(Mutex::new(Vec::with_capacity(8)));
    let _ = RELAY_INBOX.set(inbox.clone());
    thread::Builder::new()
        .name("remote-perf-relay".to_string())
        .spawn(move || {
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::net::UnixDatagram;

                let _ = std::fs::remove_file(&_socket);
                let sock = match UnixDatagram::bind(&_socket) {
                    Ok(sock) => sock,
                    Err(_) => return,
                };
                let mut buf = vec![0u8; 64 * 1024];
                while let Ok(n) = sock.recv(&mut buf) {
                    if let Ok(s) = std::str::from_utf8(&buf[..n]) {
                        if let Ok(mut guard) = inbox.lock() {
                            if guard.len() < 16 {
                                guard.push(s.to_string());
                            } else {
                                WEBHOOK_DROPPED.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        })
        .ok();
}

fn snapshot_loop() {
    let mut state = SnapshotState::default();
    let interval = Duration::from_secs(env_u64("PERF_SNAPSHOT_INTERVAL_SECS", 10));

    loop {
        let shutdown = wait_snapshot_interval(interval);
        emit_snapshot(&mut state, shutdown);
        if shutdown {
            thread::sleep(Duration::from_millis(env_u64("PERF_FINAL_EXIT_DELAY_MS", 150)));
            std::process::exit(0);
        }
    }
}

fn wait_snapshot_interval(interval: Duration) -> bool {
    let step = Duration::from_millis(200);
    let start = Instant::now();
    while start.elapsed() < interval {
        if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
            return true;
        }
        thread::sleep(step);
    }
    SHUTDOWN_REQUESTED.load(Ordering::Relaxed)
}

fn emit_snapshot(state: &mut SnapshotState, final_snapshot: bool) {
    let local = build_snapshot(state);
    let relayed = drain_relay_inbox();

    if WEBHOOK_TX.get().is_some() {
        let body = json!({
            "event": if final_snapshot { "remote_perf_final_snapshot" } else { "remote_perf_snapshot" },
            "timestamp": unix_timestamp_millis(),
            "aggregator": instance_id(),
            "shutdown": final_snapshot,
            "local": local,
            "relayed": relayed,
            "bottlenecks": bottlenecks(),
        })
        .to_string();
        if final_snapshot {
            send_webhook_direct(&body);
        } else {
            enqueue(body);
        }
    } else if RELAY_TARGET.get().is_some() {
        let body = json!({
            "event": if final_snapshot { "remote_perf_final_relay" } else { "remote_perf_relay" },
            "timestamp": unix_timestamp_millis(),
            "shutdown": final_snapshot,
            "local": local,
        });
        send_relay(body.to_string());
    }
}

fn build_snapshot(state: &mut SnapshotState) -> Value {
    let now = Instant::now();
    let elapsed = now
        .duration_since(state.last_instant)
        .as_secs_f64()
        .max(0.001);
    state.last_instant = now;

    let requests = REQUESTS_TOTAL.load(Ordering::Relaxed);
    let success = REQUESTS_SUCCESS.load(Ordering::Relaxed);
    let error = REQUESTS_ERROR.load(Ordering::Relaxed);
    let window_requests = requests.saturating_sub(state.prev_requests);
    let window_success = success.saturating_sub(state.prev_success);
    let window_error = error.saturating_sub(state.prev_error);
    state.prev_requests = requests;
    state.prev_success = success;
    state.prev_error = error;

    let cpu_now = read_cpu_ticks();
    let cpu_percent = cpu_percent(state.last_cpu, cpu_now, elapsed);
    state.last_cpu = cpu_now;

    let mem = read_proc_status().unwrap_or_default();
    let statm = read_proc_statm().unwrap_or_default();
    let rss_kb = if mem.rss_kb > 0 {
        mem.rss_kb
    } else {
        statm.rss_kb
    };
    let virtual_kb = if mem.virtual_kb > 0 {
        mem.virtual_kb
    } else {
        statm.virtual_kb
    };
    let memory_growth_kb = rss_kb.saturating_sub(state.last_rss_kb);
    state.last_rss_kb = rss_kb;

    let cumulative_stages = stage_stats(false, state);
    let window_stages = stage_stats(true, state);

    let current_alloc_calls = ALLOC_CALLS.load(Ordering::Relaxed);
    let current_alloc_bytes = ALLOC_BYTES.load(Ordering::Relaxed);
    let current_realloc_calls = REALLOC_CALLS.load(Ordering::Relaxed);
    let current_realloc_bytes = REALLOC_BYTES.load(Ordering::Relaxed);
    let window_alloc_calls = current_alloc_calls.saturating_sub(state.prev_alloc_calls);
    let window_alloc_bytes = current_alloc_bytes.saturating_sub(state.prev_alloc_bytes);
    let window_realloc_calls = current_realloc_calls.saturating_sub(state.prev_realloc_calls);
    let window_realloc_bytes = current_realloc_bytes.saturating_sub(state.prev_realloc_bytes);
    state.prev_alloc_calls = current_alloc_calls;
    state.prev_alloc_bytes = current_alloc_bytes;
    state.prev_realloc_calls = current_realloc_calls;
    state.prev_realloc_bytes = current_realloc_bytes;

    let epoll_wait_calls = EPOLL_WAIT_CALLS.load(Ordering::Relaxed);
    let epoll_wait_timeouts = EPOLL_WAIT_TIMEOUTS.load(Ordering::Relaxed);
    let epoll_wait_errors = EPOLL_WAIT_ERRORS.load(Ordering::Relaxed);
    let epoll_wait_events = EPOLL_WAIT_EVENTS.load(Ordering::Relaxed);
    let window_epoll_wait_calls =
        epoll_wait_calls.saturating_sub(state.prev_epoll_wait_calls);
    let window_epoll_wait_timeouts =
        epoll_wait_timeouts.saturating_sub(state.prev_epoll_wait_timeouts);
    let window_epoll_wait_errors =
        epoll_wait_errors.saturating_sub(state.prev_epoll_wait_errors);
    let window_epoll_wait_events =
        epoll_wait_events.saturating_sub(state.prev_epoll_wait_events);
    state.prev_epoll_wait_calls = epoll_wait_calls;
    state.prev_epoll_wait_timeouts = epoll_wait_timeouts;
    state.prev_epoll_wait_errors = epoll_wait_errors;
    state.prev_epoll_wait_events = epoll_wait_events;

    json!({
        "timestamp": unix_timestamp_millis(),
        "instance": instance_id(),
        "role": std::env::var("PERF_ROLE").unwrap_or_else(|_| "process".to_string()),
        "pid": std::process::id(),
        "uptime_window_secs": elapsed,
        "requests": requests,
        "requests_success": success,
        "requests_error": error,
        "window_requests": window_requests,
        "window_success": window_success,
        "window_error": window_error,
        "rps": window_requests as f64 / elapsed,
        "active_connections": ACTIVE_CONNECTIONS.load(Ordering::Relaxed),
        "bytes_received": BYTES_RECEIVED.load(Ordering::Relaxed),
        "bytes_sent": BYTES_SENT.load(Ordering::Relaxed),
        "cache_hits": CACHE_HITS.load(Ordering::Relaxed),
        "cache_misses": CACHE_MISSES.load(Ordering::Relaxed),
        "cache_hit_rate": percent(CACHE_HITS.load(Ordering::Relaxed), CACHE_HITS.load(Ordering::Relaxed) + CACHE_MISSES.load(Ordering::Relaxed)),
        "lb": {
            "accepted": LB_ACCEPTED.load(Ordering::Relaxed),
            "handoff_ok": LB_HANDOFF_OK.load(Ordering::Relaxed),
            "handoff_error": LB_HANDOFF_ERR.load(Ordering::Relaxed),
            "upstream_api1": LB_UPSTREAM_API1.load(Ordering::Relaxed),
            "upstream_api2": LB_UPSTREAM_API2.load(Ordering::Relaxed),
        },
        "socket": {
            "recv_fd_total": RECV_FD_TOTAL.load(Ordering::Relaxed),
            "spin_read_hit": SPIN_READ_HIT.load(Ordering::Relaxed),
            "spin_read_miss": SPIN_READ_MISS.load(Ordering::Relaxed),
            "spin_read_attempts": SPIN_READ_ATTEMPTS.load(Ordering::Relaxed),
            "epoll_read_fallback": EPOLL_READ_FALLBACK.load(Ordering::Relaxed),
            "recv_calls": RECV_CALLS.load(Ordering::Relaxed),
            "send_calls": SEND_CALLS.load(Ordering::Relaxed),
            "partial_writes": PARTIAL_WRITES.load(Ordering::Relaxed),
            "write_eagain": WRITE_EAGAIN.load(Ordering::Relaxed),
        },
        "epoll": {
            "wait_calls": epoll_wait_calls,
            "wait_timeouts": epoll_wait_timeouts,
            "wait_errors": epoll_wait_errors,
            "wait_events": epoll_wait_events,
            "max_events_per_wakeup": EPOLL_WAIT_MAX_EVENTS.load(Ordering::Relaxed),
            "events_per_wakeup": avg(epoll_wait_events, epoll_wait_calls.saturating_sub(epoll_wait_timeouts)),
            "wakeups_per_sec": window_epoll_wait_calls.saturating_sub(window_epoll_wait_timeouts) as f64 / elapsed,
            "timeouts_per_sec": window_epoll_wait_timeouts as f64 / elapsed,
            "errors_per_sec": window_epoll_wait_errors as f64 / elapsed,
            "window_events_per_wakeup": avg(window_epoll_wait_events, window_epoll_wait_calls.saturating_sub(window_epoll_wait_timeouts)),
            "busy_poll": {
                "supported": EPOLL_BUSY_POLL_SUPPORTED.load(Ordering::Relaxed),
                "enabled": EPOLL_BUSY_POLL_ENABLED.load(Ordering::Relaxed),
                "busy_poll_usecs": EPOLL_BUSY_POLL_USECS.load(Ordering::Relaxed),
                "busy_poll_budget": EPOLL_BUSY_POLL_BUDGET.load(Ordering::Relaxed),
                "prefer_busy_poll": EPOLL_BUSY_POLL_PREFER.load(Ordering::Relaxed),
                "set_errno": EPOLL_BUSY_POLL_SET_ERRNO.load(Ordering::Relaxed),
                "get_errno": EPOLL_BUSY_POLL_GET_ERRNO.load(Ordering::Relaxed),
            }
        },
        "stages_cumulative": cumulative_stages,
        "stages_window": window_stages,
        "hotspots_window": hotspot_stats(&window_stages),
        "memory": {
            "rss_mb": kb_to_mb(rss_kb),
            "virtual_mb": kb_to_mb(virtual_kb),
            "growth_mb": kb_to_mb(memory_growth_kb),
            "rss_per_request_bytes": memory_per_request_bytes(rss_kb, requests),
        },
        "allocations": {
            "enabled": ALLOC_PROFILING.load(Ordering::Relaxed),
            "alloc_calls": current_alloc_calls,
            "alloc_bytes": current_alloc_bytes,
            "realloc_calls": current_realloc_calls,
            "realloc_bytes": current_realloc_bytes,
            "window_alloc_calls": window_alloc_calls,
            "window_alloc_bytes": window_alloc_bytes,
            "window_realloc_calls": window_realloc_calls,
            "window_realloc_bytes": window_realloc_bytes,
            "allocations_per_request": per_request(window_alloc_calls, window_requests),
            "allocated_bytes_per_request": per_request(window_alloc_bytes, window_requests),
        },
        "cpu": {
            "percent": cpu_percent,
            "user_ticks": cpu_now.map(|c| c.user_ticks).unwrap_or(0),
            "system_ticks": cpu_now.map(|c| c.system_ticks).unwrap_or(0),
        },
        "webhook": {
            "queue_len": WEBHOOK_QUEUE_LEN.load(Ordering::Relaxed),
            "dropped": WEBHOOK_DROPPED.load(Ordering::Relaxed),
            "send_ok": WEBHOOK_SEND_OK.load(Ordering::Relaxed),
            "send_error": WEBHOOK_SEND_ERR.load(Ordering::Relaxed),
        }
    })
}

fn stage_stats(window: bool, state: &mut SnapshotState) -> Value {
    let mut stages = Vec::with_capacity(STAGE_COUNT);
    for stage in 0..STAGE_COUNT {
        let count = STAGE_COUNT_TOTAL[stage].load(Ordering::Relaxed);
        let sum = STAGE_SUM_US[stage].load(Ordering::Relaxed);
        let mut hist = [0u64; HIST_BUCKET_COUNT];
        for (bucket, slot) in hist.iter_mut().enumerate() {
            *slot = STAGE_HIST[stage][bucket].load(Ordering::Relaxed);
        }

        let (out_count, out_sum, out_hist) = if window {
            let delta_count = count.saturating_sub(state.prev_stage_counts[stage]);
            let delta_sum = sum.saturating_sub(state.prev_stage_sums[stage]);
            let mut delta_hist = [0u64; HIST_BUCKET_COUNT];
            for bucket in 0..HIST_BUCKET_COUNT {
                delta_hist[bucket] =
                    hist[bucket].saturating_sub(state.prev_stage_hist[stage][bucket]);
            }
            state.prev_stage_counts[stage] = count;
            state.prev_stage_sums[stage] = sum;
            state.prev_stage_hist[stage] = hist;
            (delta_count, delta_sum, delta_hist)
        } else {
            (count, sum, hist)
        };

        stages.push(json!({
            "stage": STAGE_NAMES[stage],
            "count": out_count,
            "total_us": out_sum,
            "avg_us": avg(out_sum, out_count),
            "p50_us": percentile_from_hist(&out_hist, 500),
            "p95_us": percentile_from_hist(&out_hist, 950),
            "p99_us": percentile_from_hist(&out_hist, 990),
            "p999_us": percentile_from_hist(&out_hist, 999),
            "max_us": if window { max_from_hist(&out_hist) } else { STAGE_MAX_US[stage].load(Ordering::Relaxed) },
        }));
    }
    json!(stages)
}

fn hotspot_stats(stages: &Value) -> Value {
    let Some(items) = stages.as_array() else {
        return json!([]);
    };
    let total: u64 = items
        .iter()
        .filter_map(|v| v.get("total_us").and_then(|x| x.as_u64()))
        .sum();
    let mut out = Vec::new();
    for item in items {
        let name = item
            .get("stage")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown");
        if name == "request_total" || name == "server_processing" {
            continue;
        }
        let total_us = item.get("total_us").and_then(|x| x.as_u64()).unwrap_or(0);
        let count = item.get("count").and_then(|x| x.as_u64()).unwrap_or(0);
        out.push(json!({
            "symbol": name,
            "samples": count,
            "total_us": total_us,
            "percent_of_measured_stage_time": percent(total_us, total),
        }));
    }
    out.sort_by(|a, b| {
        b.get("total_us")
            .and_then(|v| v.as_u64())
            .cmp(&a.get("total_us").and_then(|v| v.as_u64()))
    });
    json!(out)
}

fn bottlenecks() -> Value {
    let request_p99 = percentile_for_stage(STAGE_REQUEST_TOTAL, 990);
    let write_p99 = percentile_for_stage(STAGE_WRITE_COMPLETE, 990);
    let hit_rate = percent(
        CACHE_HITS.load(Ordering::Relaxed),
        CACHE_HITS.load(Ordering::Relaxed) + CACHE_MISSES.load(Ordering::Relaxed),
    );
    let mut items = Vec::new();
    let p99_limit = env_u64("PERF_P99_LIMIT_US", 50_000);
    if request_p99 > p99_limit {
        items.push(json!({"reason":"p99_above_limit","p99_us":request_p99,"limit_us":p99_limit}));
    }
    if write_p99 > request_p99 / 2 && request_p99 > 0 {
        items.push(json!({"reason":"write_dominates_p99","write_p99_us":write_p99,"request_p99_us":request_p99}));
    }
    if hit_rate < env_f64("PERF_CACHE_HIT_RATE_MIN", 70.0)
        && CACHE_HITS.load(Ordering::Relaxed) + CACHE_MISSES.load(Ordering::Relaxed) > 100
    {
        items.push(json!({"reason":"cache_hit_rate_low","cache_hit_rate":hit_rate}));
    }
    json!(items)
}

fn drain_relay_inbox() -> Value {
    let Some(inbox) = RELAY_INBOX.get() else {
        return json!([]);
    };
    let Ok(mut guard) = inbox.lock() else {
        return json!([]);
    };
    let raw = std::mem::take(&mut *guard);
    let parsed: Vec<Value> = raw
        .into_iter()
        .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
        .collect();
    json!(parsed)
}

fn send_relay(_body: String) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::net::UnixDatagram;
        if let Some(target) = RELAY_TARGET.get() {
            if let Ok(sock) = UnixDatagram::unbound() {
                let _ = sock.send_to(_body.as_bytes(), target);
            }
        }
    }
}

fn enqueue(body: String) {
    if let Some(tx) = WEBHOOK_TX.get() {
        match tx.try_send(body) {
            Ok(()) => {
                WEBHOOK_QUEUE_LEN.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) => {
                WEBHOOK_DROPPED.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn send_webhook_direct(body: &str) {
    let Some(webhook) = WEBHOOK_URL.get() else {
        return;
    };
    let timeout_ms = env_u64("PERF_FINAL_WEBHOOK_TIMEOUT_MS", 1500);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(timeout_ms))
        .build();
    match agent
        .post(webhook)
        .set("Content-Type", "application/json")
        .send_string(body)
    {
        Ok(_) => {
            WEBHOOK_SEND_OK.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            WEBHOOK_SEND_ERR.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn install_shutdown_signal_handlers() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::signal(libc::SIGTERM, shutdown_signal_handler as libc::sighandler_t);
        libc::signal(libc::SIGINT, shutdown_signal_handler as libc::sighandler_t);
    }
}

#[cfg(target_os = "linux")]
extern "C" fn shutdown_signal_handler(_sig: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

#[inline]
fn record_stage_hist(stage: usize, us: u64) {
    for (idx, upper) in HIST_BUCKETS_US.iter().enumerate() {
        if us <= *upper {
            STAGE_HIST[stage][idx].fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
}

fn percentile_for_stage(stage: usize, permille: u64) -> u64 {
    if stage >= STAGE_COUNT {
        return 0;
    }
    let mut hist = [0u64; HIST_BUCKET_COUNT];
    for (bucket, slot) in hist.iter_mut().enumerate() {
        *slot = STAGE_HIST[stage][bucket].load(Ordering::Relaxed);
    }
    percentile_from_hist(&hist, permille)
}

fn percentile_from_hist(hist: &[u64; HIST_BUCKET_COUNT], permille: u64) -> u64 {
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return 0;
    }
    let target = ((total * permille) + 999) / 1000;
    let mut seen = 0u64;
    for (idx, count) in hist.iter().enumerate() {
        seen += *count;
        if seen >= target {
            return HIST_BUCKETS_US[idx];
        }
    }
    *HIST_BUCKETS_US.last().unwrap()
}

fn max_from_hist(hist: &[u64; HIST_BUCKET_COUNT]) -> u64 {
    for idx in (0..HIST_BUCKET_COUNT).rev() {
        if hist[idx] > 0 {
            return HIST_BUCKETS_US[idx];
        }
    }
    0
}

#[inline]
fn elapsed_us(start: Instant) -> u64 {
    start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

#[inline]
fn update_max(cell: &AtomicU64, value: u64) {
    let mut current = cell.load(Ordering::Relaxed);
    while value > current {
        match cell.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn avg(sum: u64, count: u64) -> u64 {
    if count == 0 {
        0
    } else {
        sum / count
    }
}

fn per_request(value: u64, requests: u64) -> f64 {
    if requests == 0 {
        0.0
    } else {
        value as f64 / requests as f64
    }
}

#[derive(Clone, Copy)]
struct CpuTicks {
    user_ticks: u64,
    system_ticks: u64,
}

fn read_cpu_ticks() -> Option<CpuTicks> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let end = stat.rfind(')')?;
    let fields: Vec<&str> = stat[end + 2..].split_whitespace().collect();
    Some(CpuTicks {
        user_ticks: fields.get(11)?.parse().ok()?,
        system_ticks: fields.get(12)?.parse().ok()?,
    })
}

fn cpu_percent(prev: Option<CpuTicks>, now: Option<CpuTicks>, elapsed_secs: f64) -> f64 {
    let (prev, now) = match (prev, now) {
        (Some(prev), Some(now)) => (prev, now),
        _ => return 0.0,
    };
    let prev_total = prev.user_ticks + prev.system_ticks;
    let now_total = now.user_ticks + now.system_ticks;
    let delta = now_total.saturating_sub(prev_total) as f64;
    let ticks_per_sec = ticks_per_second();
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get() as f64)
        .unwrap_or(1.0);
    (delta / ticks_per_sec / elapsed_secs / cpus) * 100.0
}

#[derive(Default)]
struct ProcMem {
    rss_kb: u64,
    virtual_kb: u64,
}

fn read_proc_status() -> Option<ProcMem> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut mem = ProcMem::default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            mem.rss_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("VmSize:") {
            mem.virtual_kb = parse_kb(rest);
        }
    }
    Some(mem)
}

fn read_proc_statm() -> Option<ProcMem> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let mut it = statm.split_whitespace();
    let virtual_pages: u64 = it.next()?.parse().ok()?;
    let rss_pages: u64 = it.next()?.parse().ok()?;
    let page_kb = page_size_kb();
    Some(ProcMem {
        rss_kb: rss_pages * page_kb,
        virtual_kb: virtual_pages * page_kb,
    })
}

fn parse_kb(s: &str) -> u64 {
    s.split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn ticks_per_second() -> f64 {
    #[cfg(target_os = "linux")]
    {
        unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 }.max(1.0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        100.0
    }
}

fn page_size_kb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        (unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 } / 1024).max(1)
    }
    #[cfg(not(target_os = "linux"))]
    {
        4
    }
}

fn kb_to_mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}

fn memory_per_request_bytes(rss_kb: u64, requests: u64) -> u64 {
    if requests == 0 {
        0
    } else {
        (rss_kb * 1024) / requests
    }
}

fn percent(n: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (n as f64 * 100.0) / total as f64
    }
}

fn instance_id() -> String {
    format!(
        "{}:{}",
        std::env::var("PERF_ROLE").unwrap_or_else(|_| "process".to_string()),
        hostname()
    )
}

fn hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
