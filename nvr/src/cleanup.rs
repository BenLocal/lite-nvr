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
    Ok(
        nvr_db::config::get_json::<CleanupConfig>(CLEANUP_KEY, &conn)
            .await?
            .unwrap_or_default(),
    )
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
        let result = remove_segments(&expired, &conn, None).await?;
        freed = freed.saturating_add(result.freed);
        removed += result.removed;
    }

    // 2) Size rule: prune the oldest until the total is under the cap.
    if cfg.max_total_gb > 0 {
        let cap = cfg.max_total_gb as u64 * 1024 * 1024 * 1024;
        let total = record_segment::total_size(&conn).await?;
        if total > cap {
            // list() is newest-first; reverse to delete the oldest first.
            let mut segs = record_segment::list(&conn).await?;
            segs.reverse();
            let result = remove_segments(&segs, &conn, Some(total - cap)).await?;
            freed = freed.saturating_add(result.freed);
            removed += result.removed;
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

pub(crate) struct RemovalSummary {
    pub removed: usize,
    pub freed: u64,
    pub failed: usize,
}

/// Remove files first, then batch-delete only successful rows. An absent file
/// is already removed; other errors retain its row so the next pass can retry.
pub(crate) async fn remove_segments(
    segments: &[RecordSegment],
    conn: &turso::Connection,
    bytes_to_free: Option<u64>,
) -> Result<RemovalSummary> {
    let mut ids = Vec::new();
    let mut freed: u64 = 0;
    let mut failed = 0;
    for seg in segments {
        if bytes_to_free.is_some_and(|limit| freed >= limit) {
            break;
        }
        match remove_file(&seg.file_path).await {
            Ok(()) => {
                ids.push(seg.id.clone());
                freed = freed.saturating_add(seg.file_size as u64);
            }
            Err(error) => {
                failed += 1;
                log::warn!("record delete '{}' failed: {error:#}", seg.id);
            }
        }
    }
    record_segment::delete_ids(&ids, conn).await?;
    Ok(RemovalSummary {
        removed: ids.len(),
        freed,
        failed,
    })
}

async fn remove_file(path: &str) -> Result<()> {
    use anyhow::Context;
    if path.is_empty() {
        anyhow::bail!("record segment has no file path");
    }
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("delete recording file '{path}'")),
    }
}

#[cfg(test)]
#[path = "cleanup_test.rs"]
mod cleanup_test;
