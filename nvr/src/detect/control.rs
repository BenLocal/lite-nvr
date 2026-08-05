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
pub(crate) const MAX_DETECT_SAMPLE_INTERVAL_MS: u64 = 3_600_000;

/// Whether a device should have detection auto-started: enabled config on an
/// input backed by a manager `Entry::Pipe`. Native workers and supervisors
/// (`gb28181`, `onvif`, `stream`, `xiaomi`) do not expose a subscribable pipe.
pub(crate) fn should_auto_start(detect: Option<&DetectConfig>, input_type: &str) -> bool {
    detect.is_some_and(|d| d.enabled)
        && matches!(
            input_type,
            "net" | "rtsp" | "rtmp" | "file" | "v4l2" | "x11grab" | "lavfi"
        )
}

pub(crate) fn validate_detect_config(
    config: Option<&DetectConfig>,
    available_models: Option<&[String]>,
) -> anyhow::Result<()> {
    let Some(config) = config else {
        return Ok(());
    };
    if config.min_confidence.is_nan() || !(0.0..=1.0).contains(&config.min_confidence) {
        anyhow::bail!("detect min_confidence must be between 0.0 and 1.0");
    }
    if config.sample_every_ms > MAX_DETECT_SAMPLE_INTERVAL_MS {
        anyhow::bail!(
            "detect sample_every_ms must be 0 or at most {MAX_DETECT_SAMPLE_INTERVAL_MS}"
        );
    }
    if config.models.iter().any(|name| name.trim().is_empty()) {
        anyhow::bail!("detect model names must not be empty");
    }
    if let Some(available) = available_models {
        if let Some(unknown) = config
            .models
            .iter()
            .find(|name| !available.iter().any(|known| known == *name))
        {
            anyhow::bail!("detect model is not configured: {unknown}");
        }
    }
    Ok(())
}

/// Start a detection tap for `pipe`. `sample_interval_ms == 0` uses the hub
/// default. Idempotent: returns `AlreadyRunning` if a tap is already registered.
pub(crate) async fn start_tap(
    hub: &'static DetectHub,
    pipe: &str,
    want: Option<Vec<String>>,
    sample_interval_ms: u64,
    min_confidence: f32,
    auto_generation: Option<u64>,
) -> anyhow::Result<StartOutcome> {
    if auto_generation.is_none() {
        hub.cancel_auto_start(pipe);
    }
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
    let epoch = match auto_generation {
        Some(generation) => hub.register_auto_start(pipe, generation, cancel.clone()),
        None => hub.register(pipe, cancel.clone()),
    };
    let Some(epoch) = epoch else {
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
        hub.cancel_auto_start(pipe);
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

    if let Err(e) = validate_detect_config(device.config.detect.as_ref(), Some(&hub.config_names()))
    {
        log::warn!("detect: invalid config for {}: {e:#}", device.id);
        hub.cancel_auto_start(&device.id);
        hub.unregister(&device.id);
        return;
    }

    if !want_on {
        hub.cancel_auto_start(&device.id);
        hub.unregister(&device.id);
        return;
    }

    // Every caller reaches here having just (re)created the pipe, so a tap that
    // is still registered is bound to the bus that was torn down with the old
    // pipe. Free the slot now: leaving it would make the retry below see
    // `AlreadyRunning` and give up, and the stale tap dies moments later —
    // leaving the device with no detection at all.
    hub.cancel_auto_start(&device.id);
    hub.unregister(&device.id);

    // want_on implies detect is Some(enabled).
    let cfg = device.config.detect.as_ref().unwrap().clone();
    let id = device.id.clone();
    let (generation, cancel) = hub.begin_auto_start(&id);
    // Spawned for two reasons: it yields, letting the `Pipe::start` task that
    // `update_pipe` just spawned publish its bus (which is usually all the wait
    // that is needed), and it keeps `ensure_device_pipe` — and with it the
    // add/update HTTP response and the boot loop — from blocking on the retries.
    tokio::spawn(async move { auto_start_with_retry(hub, id, cfg, generation, cancel).await });
}

/// Claim the tap for `id` once its pipe is subscribable, giving up after
/// [`AUTO_START_ATTEMPTS`] or as soon as the device stops wanting detection.
async fn auto_start_with_retry(
    hub: &'static DetectHub,
    id: String,
    cfg: DetectConfig,
    generation: u64,
    cancel: CancellationToken,
) {
    let want = if cfg.models.is_empty() {
        None
    } else {
        Some(cfg.models.clone())
    };
    for attempt in 1..=AUTO_START_ATTEMPTS {
        if cancel.is_cancelled() {
            return;
        }
        match start_tap(
            hub,
            &id,
            want.clone(),
            cfg.sample_every_ms,
            cfg.min_confidence,
            Some(generation),
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
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(AUTO_START_RETRY) => {}
        }
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
