//! Periodic record-segment retention cleanup. The policy lives in the KV config
//! (`record_cleanup`, editable from the dashboard Settings page) and is applied
//! by a background worker: delete segments older than `max_age_days`, then, if a
//! total-size cap is set, prune the oldest until the total is under it. Each
//! removal drops both the file and the DB row. Disabled by default (a no-op).

use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use nvr_db::record_segment::{self, RecordSegment};

use crate::db::app_db_conn;

/// KV config key for the retention policy.
const CLEANUP_KEY: &str = "record_cleanup";
/// Delay before the first pass so startup isn't contended.
const STARTUP_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    /// Master switch; when false the worker does nothing.
    #[serde(default)]
    pub enabled: bool,
    /// Delete segments older than this many days. 0 disables the age rule.
    #[serde(default)]
    pub max_age_days: u32,
    /// Keep the total recording size under this many GiB, pruning the oldest
    /// segments first. 0 disables the size rule.
    #[serde(default)]
    pub max_total_gb: u32,
    /// How often the worker runs, in minutes (clamped to >= 1).
    #[serde(default = "default_interval")]
    pub interval_minutes: u32,
}

fn default_interval() -> u32 {
    60
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_age_days: 0,
            max_total_gb: 0,
            interval_minutes: default_interval(),
        }
    }
}

impl CleanupConfig {
    /// Normalize user input (clamp the run interval to a sane minimum).
    pub fn sanitized(mut self) -> Self {
        self.interval_minutes = self.interval_minutes.max(1);
        self
    }
}

pub async fn load_config() -> Result<CleanupConfig> {
    let conn = app_db_conn()?;
    Ok(nvr_db::config::get_json::<CleanupConfig>(CLEANUP_KEY, &conn)
        .await?
        .unwrap_or_default())
}

pub async fn save_config(cfg: &CleanupConfig) -> Result<()> {
    let conn = app_db_conn()?;
    nvr_db::config::set_json(CLEANUP_KEY, cfg, &conn).await
}

/// Spawn the retention worker; it runs until `cancel` fires. The cadence is read
/// from the config each cycle so changes take effect without a restart.
pub fn spawn_worker(cancel: CancellationToken) {
    tokio::spawn(async move {
        log::info!("record cleanup: worker started");
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(STARTUP_DELAY) => {}
        }
        loop {
            if let Err(e) = run_once().await {
                log::warn!("record cleanup: pass failed: {e:#}");
            }
            let minutes = load_config()
                .await
                .map(|c| c.interval_minutes.max(1))
                .unwrap_or(60);
            tokio::select! {
                _ = cancel.cancelled() => {
                    log::info!("record cleanup: worker stopped");
                    return;
                }
                _ = tokio::time::sleep(Duration::from_secs(minutes as u64 * 60)) => {}
            }
        }
    });
}

/// One retention pass. No-op unless enabled.
async fn run_once() -> Result<()> {
    let cfg = load_config().await?;
    if !cfg.enabled {
        return Ok(());
    }
    let conn = app_db_conn()?;
    let mut removed = 0usize;
    let mut freed: u64 = 0;

    // 1) Age rule: drop everything older than the cutoff.
    if cfg.max_age_days > 0 {
        let expired = record_segment::list_older_than_days(cfg.max_age_days, &conn).await?;
        for seg in expired {
            freed += seg.file_size as u64;
            remove_segment(&seg, &conn).await;
            removed += 1;
        }
    }

    // 2) Size rule: prune the oldest until the total is under the cap.
    if cfg.max_total_gb > 0 {
        let cap = cfg.max_total_gb as u64 * 1024 * 1024 * 1024;
        let mut total = record_segment::total_size(&conn).await?;
        if total > cap {
            // list() is newest-first; reverse to delete the oldest first.
            let mut segs = record_segment::list(&conn).await?;
            segs.reverse();
            for seg in segs {
                if total <= cap {
                    break;
                }
                let size = seg.file_size as u64;
                remove_segment(&seg, &conn).await;
                total = total.saturating_sub(size);
                freed += size;
                removed += 1;
            }
        }
    }

    if removed > 0 {
        log::info!(
            "record cleanup: removed {removed} segment(s), freed ~{} MiB",
            freed / (1024 * 1024)
        );
    }
    Ok(())
}

/// Remove one segment's file (best-effort) and its DB row.
async fn remove_segment(seg: &RecordSegment, conn: &turso::Connection) {
    remove_file(&seg.file_path).await;
    if let Err(e) = record_segment::delete(&seg.id, conn).await {
        log::warn!("record cleanup: db delete '{}' failed: {e:#}", seg.id);
    }
}

/// Best-effort file removal; a missing file is not an error.
async fn remove_file(path: &str) {
    if path.is_empty() {
        return;
    }
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("record cleanup: delete file '{path}' failed: {e:#}"),
    }
}
