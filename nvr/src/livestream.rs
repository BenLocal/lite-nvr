//! Supervisor for platform live-stream devices (`input_type == "stream"`).
//!
//! Douyin/Bilibili/Twitch-style platforms don't expose a stable pull URL: the
//! CDN address is temporary and signed. So the device stores the room/page URL
//! and this worker resolves it into a playable address via `yt-dlp` right
//! before opening the pipe — and again on every reconnect, since a stored
//! address would already be expired. Same resolve-then-play pattern as the
//! Xiaomi worker, with the extraction delegated to yt-dlp.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use media_pipe_core::{InputConfig, Pipe, PipeConfig};
use nvr_yt_dlp::{ResolvedStream, YtDlp};
use tokio_util::sync::CancellationToken;

const BACKOFF_MIN: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A session that lived at least this long counts as healthy: the next failure
/// starts the backoff over instead of continuing where it left off.
const HEALTHY_SESSION: Duration = Duration::from_secs(30);

/// Spawn the resolve → run → backoff → re-resolve supervisor loop for one
/// device. Registered in the manager as an [`Entry::Task`]; stops via `cancel`.
pub(crate) fn spawn_stream_device(
    device_id: String,
    page_url: String,
    media: Arc<rszlm::media::Media>,
    include_audio: bool,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let resolver = YtDlp::new();
        let mut backoff = BACKOFF_MIN;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            match resolver.resolve(&page_url).await {
                Ok(resolved) => {
                    log::info!(
                        "livestream {device_id}: resolved (live={}, protocol={:?})",
                        resolved.is_live,
                        resolved.protocol
                    );
                    let started = Instant::now();
                    run_session(&resolved, Arc::clone(&media), include_audio, &cancel).await;
                    if cancel.is_cancelled() {
                        break;
                    }
                    if started.elapsed() >= HEALTHY_SESSION {
                        backoff = BACKOFF_MIN;
                    }
                    log::warn!(
                        "livestream {device_id}: stream ended, re-resolving in {:?}",
                        backoff
                    );
                }
                Err(e) => {
                    log::warn!(
                        "livestream {device_id}: resolve failed: {e}, retrying in {:?}",
                        backoff
                    );
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
        log::info!("livestream {device_id}: worker stopped");
    })
}

/// Run one pipe over the freshly resolved address until the stream dies or the
/// worker is cancelled. The pipe is driven on its own task and cancelled
/// through its token so its teardown (bus stop, output drain) always runs.
///
/// Shared with the ONVIF supervisor (`crate::onvif::ingest`): both resolve an
/// address just-in-time and drive the same RTSP/network -> ZLM pipe.
pub(crate) async fn run_session(
    resolved: &ResolvedStream,
    media: Arc<rszlm::media::Media>,
    include_audio: bool,
    cancel: &CancellationToken,
) {
    let options = input_options(resolved);
    let config = PipeConfig {
        input: InputConfig::Network {
            url: resolved.url.clone(),
        },
        outputs: media_pipe_zlm::zlm_outputs(media, include_audio),
    };
    let pipe = Arc::new(Pipe::new(config));
    let pipe_for_task = Arc::clone(&pipe);
    let mut task = tokio::spawn(async move {
        pipe_for_task.start(options).await;
    });
    tokio::select! {
        _ = cancel.cancelled() => {
            pipe.cancel();
            let _ = (&mut task).await;
        }
        _ = &mut task => {}
    }
}

/// Demuxer options for the resolved address: the HTTP headers the CDN demands
/// (Referer / User-Agent / Cookie — without them picky platforms refuse the
/// pull), or the same rtsp-over-tcp policy the manager applies to rtsp inputs.
fn input_options(resolved: &ResolvedStream) -> Option<HashMap<String, String>> {
    if resolved.url.starts_with("http") {
        if resolved.http_headers.is_empty() {
            return None;
        }
        let headers: String = resolved
            .http_headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        Some(HashMap::from([("headers".to_string(), headers)]))
    } else if resolved.url.starts_with("rtsp://") {
        Some(HashMap::from([
            ("rtsp_transport".to_string(), "tcp".to_string()),
            ("stimeout".to_string(), "5000000".to_string()),
        ]))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "livestream_test.rs"]
mod livestream_test;
