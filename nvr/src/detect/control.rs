//! Shared detection start/stop plumbing. Both the REST `start` handler and
//! device-config auto-start go through `start_tap`, so the tap is built the
//! same way regardless of trigger.

use tokio_util::sync::CancellationToken;

use nvr_db::device::{DetectConfig, DeviceInfo};

use super::hub::DetectHub;

pub(crate) enum StartOutcome {
    Started,
    AlreadyRunning,
}

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

    // want_on implies detect is Some(enabled).
    let cfg = device.config.detect.as_ref().unwrap();
    let want = if cfg.models.is_empty() {
        None
    } else {
        Some(cfg.models.clone())
    };
    if let Err(e) = start_tap(
        hub,
        &device.id,
        want,
        cfg.sample_every_ms,
        cfg.min_confidence,
    )
    .await
    {
        log::warn!("detect: auto-start failed for {}: {e:#}", device.id);
    }
}

#[cfg(test)]
#[path = "control_test.rs"]
mod control_test;
