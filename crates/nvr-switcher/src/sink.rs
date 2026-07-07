//! The program output sink: ONE persistent H.264 encoder + muxer, published to
//! ZLM on a continuous CFR timeline. Whatever produces the canvas-sized frames
//! — a single-source selector ([`crate::program`]) or a multi-region compositor
//! — pushes them here. Because the encoder and muxer never restart, changing
//! what is fed in never republishes the stream, so the player never breaks.
//!
//! Shared plumbing: both `nvr-switcher`'s program bus and `nvr-compositor` build
//! on this so the "persistent seamless output" logic lives in one place.

use anyhow::Result;
use ffmpeg_bus::encoder::{Encoder, Settings};
use ffmpeg_bus::frame::RawFrame;
use ffmpeg_bus::output::AvOutput;
use ffmpeg_bus::scaler::Scaler;
use ffmpeg_bus::stream::AvStream;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::frame::Video;

/// Where/how the single program stream is encoded and published.
#[derive(Clone)]
pub struct ProgramSinkConfig {
    /// Publish URL, e.g. `rtmp://127.0.0.1:8555/live/program`.
    pub publish_url: String,
    /// Mux format for the URL, e.g. `"flv"` for RTMP.
    pub format: String,
    /// Output (canvas) width.
    pub width: u32,
    /// Output (canvas) height.
    pub height: u32,
    /// Output frame rate; also the CFR output clock.
    pub fps: u32,
    /// Optional output video bitrate (bps).
    pub bitrate: Option<u64>,
}

/// A persistent H.264 encoder + muxer on a continuous CFR timeline. Feed it
/// canvas-sized YUV420P frames via [`push`](Self::push); it never restarts, so
/// the published ZLM stream stays continuous no matter what changes upstream.
pub struct ProgramSink {
    encoder: Encoder,
    output: AvOutput,
    tick_us: i64,
    out_pts: i64,
}

impl ProgramSink {
    /// Open the encoder + muxer. `template` supplies codec/timebase context
    /// (any source's video stream); the muxed stream advertises `cfg.width x
    /// cfg.height` regardless of the template's own dimensions.
    pub fn new(template: &AvStream, cfg: &ProgramSinkConfig) -> Result<Self> {
        let settings = Settings {
            width: cfg.width,
            height: cfg.height,
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
        let encoder = Encoder::new(template, settings, Some(opts))?;

        // `for_encoder_output` copies the template's (source) dimensions;
        // overwrite them with the canvas size so the muxed stream is honest.
        let out_stream = AvStream::for_encoder_output(template, ffmpeg_next::codec::Id::H264);
        unsafe {
            let p = out_stream.parameters().as_ptr() as *mut ffmpeg_next::ffi::AVCodecParameters;
            (*p).width = cfg.width as i32;
            (*p).height = cfg.height as i32;
        }
        let mut output = AvOutput::new(&cfg.publish_url, Some(&cfg.format), None)?;
        output.add_stream(&out_stream)?;

        let tick_us = (1_000_000i64 / cfg.fps.max(1) as i64).max(1);
        Ok(Self {
            encoder,
            output,
            tick_us,
            out_pts: 0,
        })
    }

    /// Encode + mux one canvas-sized frame on the CFR timeline. `force_idr`
    /// emits a keyframe (e.g. right after a program switch, so a decoder joining
    /// or re-syncing recovers immediately).
    pub fn push(&mut self, mut frame: Video, force_idr: bool) -> Result<()> {
        frame.set_pts(Some(self.out_pts));
        if force_idr {
            frame.set_kind(ffmpeg_next::picture::Type::I);
        }
        self.encoder.send_frame(RawFrame::Video(frame.into()))?;
        while let Some(pkt) = self.encoder.encoder_receive_packet()? {
            self.output.write_packet(0, pkt)?;
        }
        self.out_pts += self.tick_us;
        Ok(())
    }

    /// Flush the encoder and finish the muxer (best-effort).
    pub fn finish(&mut self) {
        let _ = self.encoder.send_eof();
        while let Ok(Some(pkt)) = self.encoder.encoder_receive_packet() {
            let _ = self.output.write_packet(0, pkt);
        }
        let _ = self.output.finish();
    }
}

/// A cached software scaler that converts any source frame to a fixed
/// `dst_w x dst_h` YUV420P frame, rebuilding only when the source geometry
/// (width, height, pixel format) changes.
pub struct ScalerCache {
    dst_w: u32,
    dst_h: u32,
    cached: Option<((u32, u32, Pixel), Scaler)>,
}

impl ScalerCache {
    pub fn new(dst_w: u32, dst_h: u32) -> Self {
        Self {
            dst_w,
            dst_h,
            cached: None,
        }
    }

    /// Scale `src` to `dst_w x dst_h` YUV420P.
    pub fn scale(&mut self, src: &mut Video) -> Result<Video> {
        let key = (src.width(), src.height(), src.format());
        let need_new = self.cached.as_ref().map(|(k, _)| *k != key).unwrap_or(true);
        if need_new {
            let ctx = ffmpeg_next::software::scaling::Context::get(
                src.format(),
                src.width(),
                src.height(),
                Pixel::YUV420P,
                self.dst_w,
                self.dst_h,
                ffmpeg_next::software::scaling::flag::Flags::BILINEAR,
            )?;
            self.cached = Some((key, Scaler::new(ctx)));
        }
        let mut dst = Video::empty();
        self.cached.as_mut().unwrap().1.run(src, &mut dst)?;
        Ok(dst)
    }
}
