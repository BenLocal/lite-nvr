//! Per-pipe detection tap: decoded video -> sample -> RGB -> N models -> store.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ffmpeg_bus::frame::{RawFrame, RawFrameCmd, RawFrameReceiver};
use nvr_detect::{Detector, ModelResult};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use super::convert::to_rgb;
use super::hub::DetectHub;
use super::result::FrameResult;

/// Run every detector on the same RGB image concurrently (each on a blocking
/// thread, since ONNX inference is CPU-bound). Output order matches `detectors`
/// for every detector that completes; a task that fails to join (e.g. a panic)
/// is logged and omitted, so on that rare path the output can be shorter than
/// the input.
pub async fn fanout(
    detectors: &[Arc<dyn Detector>],
    rgb: Arc<Vec<u8>>,
    w: u32,
    h: u32,
) -> Vec<ModelResult> {
    let mut handles = Vec::with_capacity(detectors.len());
    for det in detectors {
        let det = det.clone();
        let rgb = rgb.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let res = det.detect(&rgb, w, h);
            let infer_ms = start.elapsed().as_secs_f64() * 1000.0;
            let name = det.name().to_string();
            match res {
                Ok(detections) => ModelResult {
                    name,
                    infer_ms,
                    detections,
                    error: None,
                },
                Err(e) => ModelResult {
                    name,
                    infer_ms,
                    detections: vec![],
                    error: Some(format!("{e:#}")),
                },
            }
        }));
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        match h.await {
            Ok(r) => out.push(r),
            Err(e) => log::warn!("detect: fanout task join error: {e}"),
        }
    }
    out
}

/// Drive one pipe's detection until `cancel` fires or the video broadcast ends.
pub async fn run(
    pipe: String,
    detectors: Vec<Arc<dyn Detector>>,
    mut video: RawFrameReceiver,
    sample_interval_ms: u64,
    hub: &'static DetectHub,
    cancel: CancellationToken,
) {
    let interval = Duration::from_millis(sample_interval_ms);
    let mut last: Option<Instant> = None;

    loop {
        let cmd = tokio::select! {
            _ = cancel.cancelled() => break,
            r = video.recv() => r,
        };
        match cmd {
            Ok(RawFrameCmd::Data(RawFrame::Video(vf))) => {
                let now = Instant::now();
                if let Some(l) = last {
                    if now.duration_since(l) < interval {
                        continue; // drop frames faster than the sample rate
                    }
                }
                last = Some(now);

                let (rgb, w, h) = match to_rgb(&vf) {
                    Ok(t) => t,
                    Err(e) => {
                        log::debug!("detect[{pipe}]: convert error: {e:#}");
                        continue;
                    }
                };
                let models = fanout(&detectors, Arc::new(rgb), w, h).await;
                hub.store(
                    &pipe,
                    FrameResult {
                        ts: chrono::Utc::now().timestamp(),
                        frame_w: w,
                        frame_h: h,
                        models,
                    },
                );
            }
            Ok(RawFrameCmd::Data(RawFrame::Audio(_))) => {}
            Ok(RawFrameCmd::EOF) => break,
            Err(RecvError::Lagged(n)) => {
                log::debug!("detect[{pipe}]: dropped {n} frames (lag)");
            }
            Err(RecvError::Closed) => break,
        }
    }
    log::info!("detect[{pipe}]: tap stopped");
}

#[cfg(test)]
#[path = "tap_test.rs"]
mod tap_test;
