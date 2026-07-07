//! The program bus: selects the active source's frames and pushes them onto the
//! shared [`ProgramSink`] (ONE persistent encoder + muxer, published to ZLM). It
//! forces an IDR on each switch; nothing here restarts on a switch, so the ZLM
//! stream is never republished.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use ffmpeg_bus::frame::RawFrame;
use ffmpeg_bus::stream::AvStream;
use tokio::sync::mpsc;

use crate::sink::{ProgramSink, ProgramSinkConfig, ScalerCache};
use crate::source::TaggedFrame;
use crate::switcher::Active;

/// Where/how the single program stream is encoded and published.
#[derive(Clone)]
pub struct ProgramConfig {
    /// Publish URL, e.g. `rtmp://127.0.0.1:8555/live/program`.
    pub publish_url: String,
    /// Mux format for the URL, e.g. `"flv"` for RTMP.
    pub format: String,
    /// Output frame rate; also the CFR output clock.
    pub fps: u32,
    /// Optional output video bitrate (bps).
    pub bitrate: Option<u64>,
}

/// Spawn the program loop on a blocking thread (encode + mux is CPU work).
pub fn spawn_program(
    cfg: ProgramConfig,
    template: AvStream,
    active: Active,
    force_idr: Arc<AtomicBool>,
    rx: mpsc::Receiver<TaggedFrame>,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::task::spawn_blocking(move || {
        let r = run_program(cfg, template, active, force_idr, rx);
        if let Err(ref e) = r {
            log::error!("program loop exited: {e:#}");
        }
        r
    })
}

fn run_program(
    cfg: ProgramConfig,
    template: AvStream,
    active: Active,
    force_idr: Arc<AtomicBool>,
    mut rx: mpsc::Receiver<TaggedFrame>,
) -> Result<()> {
    // Program resolution = the first source's resolution; every other source is
    // scaled to it. (A fixed configurable size is a later refinement.)
    let width = template.width().max(2);
    let height = template.height().max(2);
    log::info!(
        "program: publishing {width}x{height} @ {}fps ({}) to {}",
        cfg.fps,
        cfg.format,
        cfg.publish_url
    );

    let mut sink = ProgramSink::new(
        &template,
        &ProgramSinkConfig {
            publish_url: cfg.publish_url,
            format: cfg.format,
            width,
            height,
            fps: cfg.fps,
            bitrate: cfg.bitrate,
        },
    )?;
    // Scaler to the fixed program size, cached by source geometry.
    let mut scaler = ScalerCache::new(width, height);

    while let Some(tagged) = rx.blocking_recv() {
        // Only the active source contributes to the program.
        if *active.lock().unwrap() != tagged.source_id {
            continue;
        }
        let RawFrame::Video(mut rvf) = tagged.frame else {
            continue;
        };
        // Normalize to the encoder's fixed WxH + YUV420P, then push. Scaling
        // here (not inside the encoder) keeps our per-frame pict_type, so
        // forcing an IDR on the switch frame actually takes effect.
        let dst = scaler.scale(rvf.get_mut())?;
        let idr = force_idr.swap(false, Ordering::AcqRel);
        sink.push(dst, idr)?;
    }

    log::info!("program: input ended, flushing");
    sink.finish();
    Ok(())
}
