//! Background transport worker: periodically copies not-yet-uploaded record
//! segments to every enabled target, retrying failures up to a cap. Copy only —
//! local files are kept so dashboard playback is unaffected.

use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use nvr_db::transport_job::{self, STATUS_DONE, STATUS_FAILED, TransportJob};
use nvr_db::{record_segment, transport_target};

use crate::transport::backend::build_backend;
use crate::transport::config::remote_key_for;

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: i64 = 5;
const BATCH_PER_TARGET: usize = 20;

/// Spawn the transport worker; it runs until `cancel` fires.
pub fn spawn_worker(cancel: CancellationToken) {
    tokio::spawn(async move {
        log::info!(
            "transport: worker started (poll every {}s)",
            POLL_INTERVAL.as_secs()
        );
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    log::info!("transport: worker stopped");
                    return;
                }
                _ = tick.tick() => {}
            }
            if let Err(e) = sweep().await {
                log::warn!("transport: sweep failed: {e:#}");
            }
        }
    });
}

/// One pass: for each enabled target, upload a batch of pending segments.
async fn sweep() -> Result<()> {
    let conn = crate::db::app_db_conn()?;
    let targets = transport_target::list_enabled(&conn).await?;
    for target in targets {
        let backend = match build_backend(&target) {
            Ok(backend) => backend,
            Err(e) => {
                log::warn!(
                    "transport: target '{}' has invalid config: {e:#}",
                    target.name
                );
                continue;
            }
        };
        let segments = record_segment::list_needing_transport(
            &target.id,
            MAX_ATTEMPTS,
            BATCH_PER_TARGET,
            &conn,
        )
        .await?;
        for segment in segments {
            let remote_key = remote_key_for(&target, &segment);
            let existing = transport_job::get(&segment.id, &target.id, &conn).await?;
            let attempts = existing.as_ref().map(|job| job.attempts).unwrap_or(0) + 1;
            let now = chrono::Utc::now().to_rfc3339();
            let job_id = existing
                .as_ref()
                .map(|job| job.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
            let create_time = existing
                .as_ref()
                .map(|job| job.create_time.clone())
                .unwrap_or_else(|| now.clone());

            let result = backend
                .upload(std::path::Path::new(&segment.file_path), &remote_key)
                .await;
            let (status, error) = match &result {
                Ok(()) => (STATUS_DONE, String::new()),
                Err(e) => (STATUS_FAILED, format!("{e:#}")),
            };
            let job = TransportJob {
                id: job_id,
                segment_id: segment.id.clone(),
                target_id: target.id.clone(),
                status,
                attempts,
                remote_key,
                file_size: segment.file_size as i64,
                error,
                create_time,
                update_time: now,
            };
            transport_job::upsert(&job, &conn).await?;
            match result {
                Ok(()) => log::info!(
                    "transport: '{}' -> {} ({})",
                    segment.file_name,
                    target.name,
                    job.remote_key
                ),
                Err(e) => log::warn!(
                    "transport: '{}' -> {} failed (attempt {}/{}): {e:#}",
                    segment.file_name,
                    target.name,
                    attempts,
                    MAX_ATTEMPTS
                ),
            }
        }
    }
    Ok(())
}
