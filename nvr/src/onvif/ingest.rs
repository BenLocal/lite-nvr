//! ONVIF ingestion supervisor for `input_type == "onvif"` devices.
//!
//! An ONVIF camera doesn't hand you an RTSP URL directly: you query its media
//! service for a stream URI, which can change if the camera reboots or its
//! configuration changes. So the device stores the ONVIF connection config and
//! this worker resolves the RTSP URI right before opening the pipe — and again
//! on every reconnect, so an address change self-heals. Same resolve → run →
//! backoff → re-resolve shape as `livestream::spawn_stream_device`, and it
//! shares the very same `run_session`: ONVIF adds no second RTSP puller, it
//! only supplies a freshly resolved address to the existing RTSP -> ZLM pipe.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nvr_onvif::{OnvifCamera, OnvifConfig, inject_credentials};
use nvr_yt_dlp::ResolvedStream;
use tokio_util::sync::CancellationToken;

const BACKOFF_MIN: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A session that lived at least this long counts as healthy: the next failure
/// starts the backoff over instead of continuing where it left off.
const HEALTHY_SESSION: Duration = Duration::from_secs(30);

/// Spawn the resolve → run → backoff → re-resolve supervisor loop for one
/// ONVIF device. Registered in the manager as an [`crate::manager`] `Task`;
/// stops via `cancel`. A camera IP/credential change or reboot self-heals
/// because the RTSP URI is re-resolved from ONVIF on every reconnect.
pub(crate) fn spawn_onvif_device(
    device_id: String,
    cfg: OnvifConfig,
    media: Arc<rszlm::media::Media>,
    include_audio: bool,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = BACKOFF_MIN;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            match resolve_rtsp(&cfg).await {
                Ok(rtsp) => {
                    log::info!("onvif {device_id}: resolved rtsp uri");
                    let started = Instant::now();
                    // Reuse the SAME pipe driver livestream uses: wrap the RTSP
                    // URI in a ResolvedStream so `run_session`'s rtsp-over-tcp
                    // demux policy applies, exactly like a resolved rtsp pull.
                    let resolved = rtsp_stream(rtsp);
                    crate::livestream::run_session(
                        &resolved,
                        Arc::clone(&media),
                        include_audio,
                        &cancel,
                    )
                    .await;
                    if cancel.is_cancelled() {
                        break;
                    }
                    if started.elapsed() >= HEALTHY_SESSION {
                        backoff = BACKOFF_MIN;
                    }
                    log::warn!("onvif {device_id}: session ended, re-resolving in {backoff:?}");
                }
                Err(e) => {
                    log::warn!("onvif {device_id}: resolve failed: {e}, retry in {backoff:?}");
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
        log::info!("onvif {device_id}: worker stopped");
    })
}

/// Query the camera's media service for its RTSP stream URI and fold the
/// configured credentials into it (ONVIF returns the URI without them).
/// `OnvifError` is a `std::error::Error`, so `?` lifts it into `anyhow`.
async fn resolve_rtsp(cfg: &OnvifConfig) -> anyhow::Result<String> {
    let cam = OnvifCamera::connect(cfg).await?;
    let uri = cam.stream_uri(cfg.profile_token.as_deref()).await?;
    Ok(inject_credentials(&uri, &cfg.username, &cfg.password))
}

/// Present a plain RTSP URI to the shared `run_session` as a `ResolvedStream`,
/// so ONVIF drives the identical RTSP -> ZLM pipe path (including livestream's
/// rtsp-over-tcp demux options) instead of a parallel puller.
fn rtsp_stream(url: String) -> ResolvedStream {
    ResolvedStream {
        url,
        http_headers: HashMap::new(),
        is_live: true,
        title: None,
        protocol: None,
    }
}
