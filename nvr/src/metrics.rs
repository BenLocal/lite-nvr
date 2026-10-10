//! Server performance metrics (CPU / memory / network), sampled by a background
//! worker into a shared in-memory cache. The `/system/metrics` API just clones
//! the cached snapshot — it never touches `sysinfo` on the request path, so the
//! endpoint is always cheap no matter how often the dashboard polls it.
//!
//! Static host information (`/system/os`) is collected once when the worker
//! starts and cached for the life of the process.

use std::sync::{LazyLock, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{CpuRefreshKind, MINIMUM_CPU_UPDATE_INTERVAL, Networks, System};
use tokio_util::sync::CancellationToken;

/// How often the worker samples the system. Also the window over which network
/// throughput (bytes/sec) is averaged.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// A single sampled snapshot of host resource usage. All byte fields are bytes.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SystemMetrics {
    /// Overall CPU usage across all cores, 0..=100 (percent).
    pub cpu_usage: f32,
    /// Number of logical CPUs.
    pub cpu_core_count: usize,
    /// Used / total physical memory, bytes.
    pub mem_used: u64,
    pub mem_total: u64,
    /// Used / total swap, bytes.
    pub swap_used: u64,
    pub swap_total: u64,
    /// Aggregate network throughput over the last sample window, bytes/sec
    /// (loopback excluded).
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    /// Cumulative bytes since the interfaces were first listed (loopback excluded).
    pub net_rx_total: u64,
    pub net_tx_total: u64,
    /// Load average over 1 / 5 / 15 minutes (zeros where the OS has no notion).
    pub load_one: f64,
    pub load_five: f64,
    pub load_fifteen: f64,
    /// Unix-epoch millisecond timestamp of this sample; 0 before the first one.
    pub sampled_at_ms: u64,
}

static CACHE: LazyLock<RwLock<SystemMetrics>> =
    LazyLock::new(|| RwLock::new(SystemMetrics::default()));

/// The latest sampled snapshot (a cheap clone; never blocks on `sysinfo`). All
/// fields are zero until the worker has taken its first sample.
pub fn snapshot() -> SystemMetrics {
    CACHE.read().unwrap().clone()
}

/// Static host information, collected once at startup.
#[derive(Clone, Debug, Serialize)]
pub struct OsInfo {
    pub host_name: Option<String>,
    /// OS name, e.g. "Ubuntu" / "Buildroot".
    pub os_name: Option<String>,
    pub os_version: Option<String>,
    /// e.g. "Linux 24.04 Ubuntu 24.04 LTS".
    pub long_os_version: Option<String>,
    pub kernel_version: Option<String>,
    /// CPU architecture, e.g. "x86_64" / "aarch64".
    pub arch: String,
    /// Board model from the device tree (ARM boards), e.g. "Rockchip RK3588 EVB1".
    pub board_model: Option<String>,
    /// CPU model; empty on platforms that do not report one (many ARM SoCs).
    pub cpu_brand: Option<String>,
    pub cpu_core_count: usize,
    pub cpu_physical_core_count: Option<usize>,
    /// Physical memory / swap, bytes.
    pub mem_total: u64,
    pub swap_total: u64,
    /// Unix-epoch seconds the host booted; uptime = now - boot_time.
    pub boot_time: u64,
    pub nvr_version: String,
}

static OS_INFO: OnceLock<OsInfo> = OnceLock::new();

/// The startup-collected host information (collected on first use if the
/// worker has not reached it yet; never recollected).
pub fn os_info() -> OsInfo {
    OS_INFO.get_or_init(collect_os_info).clone()
}

fn collect_os_info() -> OsInfo {
    let mut sys = System::new();
    sys.refresh_cpu_list(CpuRefreshKind::nothing());
    sys.refresh_memory();
    let cpu_brand = sys
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_string())
        .filter(|brand| !brand.is_empty());
    OsInfo {
        host_name: System::host_name(),
        os_name: System::name(),
        os_version: System::os_version(),
        long_os_version: System::long_os_version(),
        kernel_version: System::kernel_version(),
        arch: System::cpu_arch(),
        board_model: std::fs::read("/proc/device-tree/model")
            .ok()
            .and_then(|raw| device_tree_string(&raw)),
        cpu_brand,
        cpu_core_count: sys.cpus().len(),
        cpu_physical_core_count: sys.physical_core_count(),
        mem_total: sys.total_memory(),
        swap_total: sys.total_swap(),
        boot_time: System::boot_time(),
        nvr_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// A device-tree string property: NUL-terminated (possibly NUL-padded) text.
fn device_tree_string(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Sum RX/TX (per-window and cumulative) across every interface except loopback.
fn network_totals(networks: &Networks) -> (u64, u64, u64, u64) {
    let (mut rx, mut tx, mut rx_total, mut tx_total) = (0u64, 0u64, 0u64, 0u64);
    for (name, data) in networks {
        if name == "lo" {
            continue;
        }
        rx += data.received();
        tx += data.transmitted();
        rx_total += data.total_received();
        tx_total += data.total_transmitted();
    }
    (rx, tx, rx_total, tx_total)
}

/// Spawn the sampling worker. `sysinfo`'s handles are not `Sync` and the sampler
/// is a permanent loop, so it lives on its own OS thread rather than the tokio
/// pool. It runs until `cancel` fires (and the process exit on shutdown reaps it
/// regardless).
pub fn spawn_worker(cancel: CancellationToken) {
    let spawned = std::thread::Builder::new()
        .name("metrics".into())
        .spawn(move || {
            log::info!("metrics: worker started");
            let os = os_info();
            log::info!(
                "metrics: host {} {} ({}), {} cpus",
                os.long_os_version.as_deref().unwrap_or("unknown os"),
                os.kernel_version.as_deref().unwrap_or(""),
                os.board_model.as_deref().unwrap_or(&os.arch),
                os.cpu_core_count
            );
            let mut sys = System::new();
            let mut networks = Networks::new_with_refreshed_list();

            // CPU usage is a diff between two refreshes; prime it once so the
            // first stored sample is meaningful rather than zero.
            sys.refresh_cpu_usage();
            std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);

            let secs = SAMPLE_INTERVAL.as_secs_f64().max(0.001);
            loop {
                if cancel.is_cancelled() {
                    log::info!("metrics: worker stopped");
                    return;
                }

                sys.refresh_cpu_usage();
                sys.refresh_memory();
                networks.refresh(true);

                let (rx, tx, rx_total, tx_total) = network_totals(&networks);
                let load = System::load_average();
                let sample = SystemMetrics {
                    cpu_usage: sys.global_cpu_usage(),
                    cpu_core_count: sys.cpus().len(),
                    mem_used: sys.used_memory(),
                    mem_total: sys.total_memory(),
                    swap_used: sys.used_swap(),
                    swap_total: sys.total_swap(),
                    net_rx_bps: (rx as f64 / secs) as u64,
                    net_tx_bps: (tx as f64 / secs) as u64,
                    net_rx_total: rx_total,
                    net_tx_total: tx_total,
                    load_one: load.one,
                    load_five: load.five,
                    load_fifteen: load.fifteen,
                    sampled_at_ms: now_ms(),
                };
                *CACHE.write().unwrap() = sample;

                // Sleep the sample interval, but wake early (in small steps) if
                // we're asked to cancel so shutdown isn't blocked.
                let mut slept = Duration::ZERO;
                while slept < SAMPLE_INTERVAL {
                    if cancel.is_cancelled() {
                        log::info!("metrics: worker stopped");
                        return;
                    }
                    let step = Duration::from_millis(250).min(SAMPLE_INTERVAL - slept);
                    std::thread::sleep(step);
                    slept += step;
                }
            }
        });
    if let Err(e) = spawned {
        log::error!("metrics: failed to start worker thread: {e}");
    }
}

#[cfg(test)]
#[path = "metrics_test.rs"]
mod metrics_test;
