use std::time::Duration;

use ffmpeg_next::{Dictionary, Rational, picture};
use tokio_util::sync::CancellationToken;

use crate::{
    frame::{RawFrame, RawFrameCmd, RawFrameReceiver},
    packet::{RawPacket, RawPacketCmd, RawPacketReceiver, RawPacketSender},
    scaler::Scaler,
    stream::AvStream,
};

pub enum EncoderType {
    Video(ffmpeg_next::codec::encoder::Video),
    Audio(ffmpeg_next::codec::encoder::Audio),
}

impl EncoderType {
    pub fn send_frame(&mut self, frame: RawFrame, frame_index: i64) -> anyhow::Result<()> {
        match (self, frame) {
            (EncoderType::Video(encoder), RawFrame::Video(mut frame)) => {
                let frame = frame.get_mut();
                // todo
                if frame_index % 5 == 0 {
                    frame.set_kind(picture::Type::I);
                }
                // Set PTS if not already set
                if frame.pts().is_none() {
                    frame.set_pts(Some(frame_index));
                }
                encoder.send_frame(frame)?;
            }
            (EncoderType::Audio(encoder), RawFrame::Audio(mut frame)) => {
                let frame = frame.get_mut();
                encoder.send_frame(frame)?;
            }
            _ => anyhow::bail!("invalid frame type"),
        };

        Ok(())
    }

    pub fn send_eof(&mut self) -> anyhow::Result<()> {
        match self {
            EncoderType::Video(encoder) => encoder.send_eof()?,
            EncoderType::Audio(encoder) => encoder.send_eof()?,
        }
        Ok(())
    }

    pub fn encoder_receive_packet(
        &mut self,
        time_base: Rational,
    ) -> anyhow::Result<Option<RawPacket>> {
        let mut packet = ffmpeg_next::codec::packet::Packet::empty();
        let encode_result = match self {
            EncoderType::Video(encoder) => encoder.receive_packet(&mut packet),
            EncoderType::Audio(encoder) => encoder.receive_packet(&mut packet),
        };

        match encode_result {
            Ok(()) => Ok(Some(RawPacket::from((packet, time_base)))),
            Err(ffmpeg_next::Error::Other { errno })
                if errno == ffmpeg_next::util::error::EAGAIN =>
            {
                Ok(None)
            }
            Err(ffmpeg_next::Error::Eof) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub keyframe_interval: u64,
    pub codec: Option<String>,
    pub pixel_format: ffmpeg_next::format::Pixel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            keyframe_interval: 25,
            codec: Some("libx264".to_string()),
            pixel_format: ffmpeg_next::format::Pixel::YUV420P,
        }
    }
}

/// Returns a pixel format suitable for libx264. Source formats not supported by libx264 (e.g. rgb24)
/// are mapped to YUV420P; the encoder will use its internal scaler to convert when sending frames.
pub fn pixel_format_for_libx264(source: ffmpeg_next::format::Pixel) -> ffmpeg_next::format::Pixel {
    use ffmpeg_next::format::Pixel;
    match source {
        Pixel::RGB24 | Pixel::BGR24 => Pixel::YUV420P,
        _ => source,
    }
}

pub struct Encoder {
    stream: AvStream,
    inner: EncoderType,
    encoder_time_base: Rational,
    interleaved: bool,
    frame_index: i64,
    scaler: Option<Scaler>,
}

impl Encoder {
    pub fn new(
        stream: &AvStream,
        settings: Settings,
        options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        let encoder_context = match settings.codec {
            Some(codec) => {
                let codec = ffmpeg_next::encoder::find_by_name(&codec)
                    .ok_or(anyhow::anyhow!("codec not found"))?;
                ffmpeg_next::codec::Context::new_with_codec(codec)
            }
            None => ffmpeg_next::codec::Context::new(),
        };

        let mut encoder = encoder_context.encoder().video()?;
        encoder.set_width(settings.width);
        encoder.set_height(settings.height);
        encoder.set_format(settings.pixel_format);
        encoder.set_frame_rate(Some(stream.rate()));
        encoder.set_time_base(ffmpeg_next::util::mathematics::rescale::TIME_BASE);

        // Encoding efficiency: preset (ultrafast=fastest), tune zerolatency; caller may pass options via EncodeConfig.
        let need_defaults = options.is_none();
        let mut opts = options.unwrap_or_default();
        if need_defaults {
            opts.set("preset", "ultrafast");
            opts.set("tune", "zerolatency");
        }
        let encoder = encoder.open_with(opts)?;
        let encoder_time_base: Rational = unsafe { (*encoder.0.as_ptr()).time_base.into() };

        Ok(Self {
            stream: stream.clone(),
            inner: EncoderType::Video(encoder),
            encoder_time_base: encoder_time_base,
            interleaved: false,
            frame_index: 0,
            scaler: None,
        })
    }

    fn rescale_pts(&self, pts: i64) -> i64 {
        let src_tb = self.stream.time_base();
        let dst_tb = self.encoder_time_base;
        let num = src_tb.0 as i128 * dst_tb.1 as i128;
        let den = src_tb.1 as i128 * dst_tb.0 as i128;
        if den == 0 {
            pts
        } else {
            (pts as i128 * num / den) as i64
        }
    }

    /// PTS in encoder time_base when input has no PTS (e.g. decoder output AV_NOPTS_VALUE).
    /// Ensures monotonic timestamps at correct frame rate so downstream (e.g. ZLMediaKit) does not clear cache.
    fn pts_for_frame_index(&self, frame_index: i64) -> i64 {
        let rate = self.stream.rate();
        let tb = self.encoder_time_base;
        if rate.0 <= 0 {
            return frame_index;
        }
        // 1 frame = 1/fps seconds; in encoder time_base: frame_index * (tb.den / (fps * tb.num))
        let fps_num = rate.0 as i64;
        let fps_den = rate.1 as i64;
        let tb_num = tb.0 as i64;
        let tb_den = tb.1 as i64;
        (frame_index * tb_den * fps_den) / (tb_num * fps_num)
    }

    pub fn send_frame(&mut self, mut frame: RawFrame) -> anyhow::Result<()> {
        let sending_frame = match (&mut frame, &self.inner) {
            (RawFrame::Video(f), EncoderType::Video(e)) => {
                let f = f.get_mut();

                if let Some(pts) = f.pts() {
                    let new_pts = self.rescale_pts(pts);
                    f.set_pts(Some(new_pts));
                } else {
                    // No PTS from decoder/input: set PTS in encoder time_base so output is correct fps
                    f.set_pts(Some(self.pts_for_frame_index(self.frame_index)));
                }

                if f.format() != e.format() {
                    if self.scaler.is_none() {
                        self.scaler =
                            Some(Scaler::new(ffmpeg_next::software::scaling::Context::get(
                                f.format(),
                                f.width(),
                                f.height(),
                                e.format(),
                                e.width(),
                                e.height(),
                                ffmpeg_next::software::scaling::flag::Flags::empty(),
                            )?));
                    }

                    let mut converted = ffmpeg_next::frame::Video::empty();
                    self.scaler.as_mut().unwrap().run(f, &mut converted)?;
                    converted.set_pts(f.pts());
                    Some(RawFrame::Video(converted.into()))
                } else {
                    None
                }
            }
            (RawFrame::Audio(_), EncoderType::Audio(_)) => None,
            _ => None,
        };

        if let Some(converted) = sending_frame {
            self.inner.send_frame(converted, self.frame_index)?;
        } else {
            self.inner.send_frame(frame, self.frame_index)?;
        }
        self.frame_index += 1;
        Ok(())
    }

    pub fn send_eof(&mut self) -> anyhow::Result<()> {
        self.inner.send_eof()
    }

    pub fn encoder_receive_packet(&mut self) -> anyhow::Result<Option<RawPacket>> {
        let rate = self.stream.rate();
        let mut pkt = self.inner.encoder_receive_packet(self.encoder_time_base)?;

        if let Some(ref mut p) = pkt {
            if rate.0 > 0 {
                let duration = 1_000_000i64 * rate.1 as i64 / rate.0 as i64;
                p.set_duration(duration);
            }
        }
        Ok(pkt)
    }
}

pub struct EncoderTask {
    cancel: CancellationToken,
    raw_chan: RawPacketSender,
}

impl EncoderTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        /// Encoder output = encoded packets (small). Moderate capacity for bursts.
        const PACKET_CHAN_CAP: usize = 64;
        let (sender, _) = tokio::sync::broadcast::channel(PACKET_CHAN_CAP);

        Self {
            cancel,
            raw_chan: sender,
        }
    }

    pub fn subscribe(&self) -> RawPacketReceiver {
        self.raw_chan.subscribe()
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub async fn start(&self, encoder: Encoder, mut encoder_receiver: RawFrameReceiver) {
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();
        log::info!(
            "encoder loop started, stream index: {}",
            encoder.stream.index()
        );
        /// Bounded queue: when encoder is slower than producer, back-pressure instead of unbounded growth (OOM).
        const FRAME_QUEUE_BOUND: usize = 128;
        /// Log "queue full" at most every N drops; use debug level so info logs stay clean.
        const DROP_LOG_INTERVAL: u64 = 120;
        tokio::spawn(async move {
            let (tx, rx) = std::sync::mpsc::sync_channel::<RawFrameCmd>(FRAME_QUEUE_BOUND);
            let handle_cancel = cancel_clone.clone();
            let handle = tokio::task::spawn_blocking(move || {
                Self::encoder_loop(encoder, handle_cancel, rx, sender_clone)
            });
            let mut dropped_count: u64 = 0;
            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        break;
                    }
                    Ok(frame) = encoder_receiver.recv() => {
                        let is_eof = matches!(&frame, RawFrameCmd::EOF);
                        let ok = if is_eof {
                            tx.send(frame).is_ok()
                        } else {
                            match tx.try_send(frame) {
                                Ok(()) => true,
                                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                                    dropped_count += 1;
                                    if dropped_count % DROP_LOG_INTERVAL == 1 {
                                        log::debug!(
                                            "encoder frame queue full, dropped {} frames (back-pressure)",
                                            dropped_count
                                        );
                                    }
                                    true
                                }
                                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
                            }
                        };
                        if !ok {
                            break;
                        }
                    }
                }
            }
            let _ = handle.await;
            log::info!("encoder task finished");
        });
    }

    fn encoder_loop(
        mut encoder: Encoder,
        cancel: CancellationToken,
        rx: std::sync::mpsc::Receiver<RawFrameCmd>,
        out: RawPacketSender,
    ) {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let mut eof = false;
            match rx.recv_timeout(Duration::from_millis(1)) {
                Ok(frame) => {
                    match frame {
                        RawFrameCmd::Data(frame) => {
                            if let Err(e) = encoder.send_frame(frame) {
                                eprintln!("send packet error: {}", e);
                                continue;
                            }
                        }
                        RawFrameCmd::EOF => {
                            if let Err(e) = encoder.send_eof() {
                                eprintln!("send eof error: {}", e);
                            }
                            eof = true;
                        }
                    };

                    'outer: loop {
                        match encoder.encoder_receive_packet() {
                            Ok(Some(packet)) => {
                                let _ = out.send(RawPacketCmd::Data(packet));
                            }
                            Ok(None) => {
                                break 'outer;
                            }
                            Err(e) => {
                                eprintln!("receive packet error: {}", e);
                                break 'outer;
                            }
                        }
                    }

                    if eof {
                        break;
                    }
                }
                Err(_) => (),
            }
        }

        println!(
            "end of av encode task loop, stream base_time: {:#?}, encoder_time_base: {:#?}",
            encoder.stream.time_base(),
            encoder.encoder_time_base
        );
        let _ = out.send(RawPacketCmd::EOF);
    }
}
