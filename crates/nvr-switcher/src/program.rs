//! The program bus: ONE persistent encoder + muxer, published to ZLM. It
//! consumes tagged frames from every source, encodes only the active source's
//! frames onto a continuous CFR timeline, and forces an IDR on each switch.
//! Nothing here restarts on a switch, so the ZLM stream is never republished.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use ffmpeg_bus::encoder::{Encoder, Settings};
use ffmpeg_bus::frame::RawFrame;
use ffmpeg_bus::output::AvOutput;
use ffmpeg_bus::scaler::Scaler;
use ffmpeg_bus::stream::AvStream;
use ffmpeg_next::format::Pixel;
use tokio::sync::mpsc;

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

    // Persistent encoder, fixed WxH + YUV420P — no source can change the
    // output's codec parameters, so players never renegotiate.
    let settings = Settings {
        width,
        height,
        keyframe_interval: cfg.fps.max(1) as u64,
        codec: Some("h264".to_string()),
        pixel_format: Pixel::YUV420P,
    };
    let mut opts = ffmpeg_next::Dictionary::new();
    opts.set("preset", "ultrafast");
    opts.set("tune", "zerolatency");
    if let Some(b) = cfg.bitrate {
        opts.set("b", &b.to_string());
    }
    let mut encoder = Encoder::new(&template, settings, Some(opts))?;

    // Persistent muxer -> ZLM. Never torn down while switching.
    let out_stream = AvStream::for_encoder_output(&template, ffmpeg_next::codec::Id::H264);
    let mut output = AvOutput::new(&cfg.publish_url, Some(&cfg.format), None)?;
    output.add_stream(&out_stream)?;
    log::info!(
        "program: publishing {}x{} @ {}fps ({}) to {}",
        width,
        height,
        cfg.fps,
        cfg.format,
        cfg.publish_url
    );

    // Our own continuous CFR clock, in the encoder's microsecond time base.
    let tick = (1_000_000i64 / cfg.fps.max(1) as i64).max(1);
    let mut out_pts: i64 = 0;
    // Scaler cached by source (w, h, pixel format).
    let mut scaler: Option<((u32, u32, Pixel), Scaler)> = None;

    while let Some(tagged) = rx.blocking_recv() {
        // Only the active source contributes to the program.
        if *active.lock().unwrap() != tagged.source_id {
            continue;
        }
        let RawFrame::Video(mut rvf) = tagged.frame else {
            continue;
        };

        // Normalize the source frame to the encoder's fixed WxH + YUV420P.
        // Doing the scaling here (not inside the encoder) means our per-frame
        // pict_type survives to force an IDR on the switch frame.
        let mut dst = ffmpeg_next::frame::Video::empty();
        {
            let src = rvf.get_mut();
            let key = (src.width(), src.height(), src.format());
            let need_new = scaler.as_ref().map(|(k, _)| *k != key).unwrap_or(true);
            if need_new {
                let ctx = ffmpeg_next::software::scaling::Context::get(
                    src.format(),
                    src.width(),
                    src.height(),
                    Pixel::YUV420P,
                    width,
                    height,
                    ffmpeg_next::software::scaling::flag::Flags::BILINEAR,
                )?;
                scaler = Some((key, Scaler::new(ctx)));
            }
            scaler.as_mut().unwrap().1.run(src, &mut dst)?;
        }

        // Continuous CFR timeline — source PTS is ignored, so a switch never
        // causes a timestamp jump.
        dst.set_pts(Some(out_pts));
        out_pts += tick;

        // Force an IDR on the first frame after a switch (and on startup).
        if force_idr.swap(false, Ordering::AcqRel) {
            dst.set_kind(ffmpeg_next::picture::Type::I);
        }

        encoder.send_frame(RawFrame::Video(dst.into()))?;
        while let Some(pkt) = encoder.encoder_receive_packet()? {
            output.write_packet(0, pkt)?;
        }
    }

    log::info!("program: input ended, flushing");
    let _ = encoder.send_eof();
    while let Ok(Some(pkt)) = encoder.encoder_receive_packet() {
        let _ = output.write_packet(0, pkt);
    }
    let _ = output.finish();
    Ok(())
}
