//! Shared detection start/stop plumbing. Both the REST `start` handler and
//! device-config auto-start go through `start_tap`, so the tap is built the
//! same way regardless of trigger.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use nvr_db::device::{DetectConfig, DeviceInfo};

use super::hub::DetectHub;

pub(crate) enum StartOutcome {
    Started,
    AlreadyRunning,
}

/// How long auto-start waits for a freshly created pipe to publish its bus.
/// `manager::update_pipe` spawns `Pipe::start`, so `subscribe_video` reports
/// "pipe not started" for the first moments of a pipe's life — and for an RTSP
/// source, until the demuxer has actually connected and read a stream header.
const AUTO_START_ATTEMPTS: u32 = 30;
const AUTO_START_RETRY: Duration = Duration::from_secs(1);

/// Whether a device should have detection auto-started: enabled config on a
/// pipe-backed device. gb28181 has no pipe, so it never qualifies.
pub(crate) fn should_auto_start(detect: Option<&DetectConfig>, input_type: &str) -> bool {
    detect.is_some_and(|d| d.enabled) && input_type != "gb28181"
}

/// Start a detection tap for `pipe`. `sample_interval_ms == 0` uses the hub
/// default. Idempotent: returns `AlreadyRunning` if a tap is already registered.
pub(crate) async fn start_tap(
    hub: &'static DetectHub,
    pipe: &str,
    want: Option<Vec<String>>,
    sample_interval_ms: u64,
    min_confidence: f32,
) -> anyhow::Result<StartOutcome> {
    if hub.is_running(pipe) {
        return Ok(StartOutcome::AlreadyRunning);
    }
    let handle = crate::manager::get_pipe(pipe)
        .await
        .ok_or_else(|| anyhow::anyhow!("pipe not found"))?;
    let video = handle
        .subscribe_video()
        .await
        .map_err(|e| anyhow::anyhow!("no video: {e:#}"))?;

    let all = hub.detectors().await?;
    let detectors = hub.detectors_named(&all, &want);
    if detectors.is_empty() {
        anyhow::bail!("no matching models");
    }

    let cancel = CancellationToken::new();
    let Some(epoch) = hub.register(pipe, cancel.clone()) else {
        return Ok(StartOutcome::AlreadyRunning);
    };
    let interval = if sample_interval_ms > 0 {
        sample_interval_ms
    } else {
        hub.sample_interval_ms()
    };
    tokio::spawn(super::tap::run(
        pipe.to_string(),
        detectors,
        video,
        interval,
        hub,
        epoch,
        cancel,
        min_confidence,
    ));
    Ok(StartOutcome::Started)
}

/// Stop a running tap for `pipe` (idempotent no-op if not running / hub down).
pub(crate) fn stop_detection(pipe: &str) {
    if let Some(hub) = DetectHub::get() {
        hub.unregister(pipe);
    }
}

/// Reconcile a device's persisted detection config against the running tap:
/// start when enabled + pipe-backed, stop otherwise.
pub(crate) async fn reconcile_detection(device: &DeviceInfo) {
    let want_on = should_auto_start(device.config.detect.as_ref(), &device.input_type);

    let Some(hub) = DetectHub::get() else {
        if want_on {
            log::warn!(
                "detect: hub not initialized; skipping auto-start for {}",
                device.id
            );
        }
        return;
    };

    if !want_on {
        hub.unregister(&device.id);
        return;
    }

    // Every caller reaches here having just (re)created the pipe, so a tap that
    // is still registered is bound to the bus that was torn down with the old
    // pipe. Free the slot now: leaving it would make the retry below see
    // `AlreadyRunning` and give up, and the stale tap dies moments later —
    // leaving the device with no detection at all.
    hub.unregister(&device.id);

    // want_on implies detect is Some(enabled).
    let cfg = device.config.detect.as_ref().unwrap().clone();
    let id = device.id.clone();
    // Spawned for two reasons: it yields, letting the `Pipe::start` task that
    // `update_pipe` just spawned publish its bus (which is usually all the wait
    // that is needed), and it keeps `ensure_device_pipe` — and with it the
    // add/update HTTP response and the boot loop — from blocking on the retries.
    tokio::spawn(async move { auto_start_with_retry(hub, id, cfg).await });
}

/// Claim the tap for `id` once its pipe is subscribable, giving up after
/// [`AUTO_START_ATTEMPTS`] or as soon as the device stops wanting detection.
async fn auto_start_with_retry(hub: &'static DetectHub, id: String, cfg: DetectConfig) {
    let want = if cfg.models.is_empty() {
        None
    } else {
        Some(cfg.models.clone())
    };
    for attempt in 1..=AUTO_START_ATTEMPTS {
        match start_tap(
            hub,
            &id,
            want.clone(),
            cfg.sample_every_ms,
            cfg.min_confidence,
        )
        .await
        {
            Ok(StartOutcome::Started) => {
                log::info!("detect: auto-started {id} (attempt {attempt})");
                return;
            }
            Ok(StartOutcome::AlreadyRunning) => return,
            Err(e) if attempt == AUTO_START_ATTEMPTS => {
                log::warn!("detect: auto-start gave up for {id} after {attempt} attempts: {e:#}");
                return;
            }
            Err(e) => {
                log::debug!("detect: auto-start attempt {attempt} for {id}: {e:#}");
            }
        }
        tokio::time::sleep(AUTO_START_RETRY).await;
        // Re-read rather than trusting the config we were handed: the user can
        // disable detection or delete the device while we are still waiting.
        if !should_keep_retrying(reread_device(&id).await.as_ref()) {
            log::info!("detect: auto-start for {id} abandoned (config changed while waiting)");
            return;
        }
    }
}

/// Whether a pending auto-start retry is still wanted. A device that vanished
/// mid-wait (removed) or had detection turned off must not get a tap.
fn should_keep_retrying(device: Option<&DeviceInfo>) -> bool {
    device.is_some_and(|d| should_auto_start(d.config.detect.as_ref(), &d.input_type))
}

async fn reread_device(id: &str) -> Option<DeviceInfo> {
    let conn = crate::db::app_db_conn().ok()?;
    nvr_db::device::get(id, &conn).await.ok().flatten()
}

#[cfg(test)]
#[path = "control_test.rs"]
mod control_test;
