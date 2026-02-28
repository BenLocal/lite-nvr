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

pub use crate::hw::{pixel_format_for_encoder, pixel_format_for_libx264};
use crate::hw::find_hw_encoder;

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
        let (encoder_context, selected_codec_name) = match settings.codec {
            Some(ref codec) => {
                // Try hardware encoder first, then fall back to software encoder.
                if let Some(hw_codec) = find_hw_encoder(codec) {
                    let hw_name = hw_codec.name().to_string();
                    log::info!("attempting hardware encoder: {}", hw_name);
                    (
                        ffmpeg_next::codec::Context::new_with_codec(hw_codec),
                        hw_name,
                    )
                } else {
                    log::info!("no hardware encoder found, using software encoder: {}", codec);
                    let sw_codec = ffmpeg_next::encoder::find_by_name(codec)
                        .ok_or(anyhow::anyhow!("codec not found: {}", codec))?;
                    (
                        ffmpeg_next::codec::Context::new_with_codec(sw_codec),
                        codec.clone(),
                    )
                }
            }
            None => (ffmpeg_next::codec::Context::new(), String::new()),
        };

        // Try to open the encoder; if hardware encoder fails, retry with software.
        let open_encoder = |ctx: ffmpeg_next::codec::Context,
                            opts: Option<Dictionary>,
                            settings: &Settings|
         -> anyhow::Result<ffmpeg_next::codec::encoder::Video> {
            let mut encoder = ctx.encoder().video()?;
            encoder.set_width(settings.width);
            encoder.set_height(settings.height);
            encoder.set_format(settings.pixel_format);
            encoder.set_frame_rate(Some(stream.rate()));
            encoder.set_time_base(ffmpeg_next::util::mathematics::rescale::TIME_BASE);

            let need_defaults = opts.is_none();
            let mut opts = opts.unwrap_or_default();
            if need_defaults {
                opts.set("preset", "ultrafast");
                opts.set("tune", "zerolatency");
            }
            let encoder = encoder.open_with(opts)?;
            Ok(encoder)
        };

        let encoder = match open_encoder(
            encoder_context,
            options.clone(),
            &settings,
        ) {
            Ok(enc) => {
                log::info!("encoder opened successfully: {}", selected_codec_name);
                enc
            }
            Err(e) => {
                // If it was a hardware encoder attempt, fall back to software
                if let Some(ref codec) = settings.codec {
                    let is_hw = selected_codec_name != *codec;
                    if is_hw {
                        log::warn!(
                            "hardware encoder {} failed: {}, falling back to {}",
                            selected_codec_name,
                            e,
                            codec
                        );
                        let sw_codec = ffmpeg_next::encoder::find_by_name(codec)
                            .ok_or(anyhow::anyhow!("codec not found: {}", codec))?;
                        let sw_ctx = ffmpeg_next::codec::Context::new_with_codec(sw_codec);
                        let enc = open_encoder(sw_ctx, options, &settings)?;
                        log::info!("encoder opened successfully (fallback): {}", codec);
                        enc
                    } else {
                        return Err(e);
                    }
                } else {
                    return Err(e);
                }
            }
        };

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

    pub fn send_frame(&mut self, mut frame: RawFrame) -> anyhow::Result<()> {
        let sending_frame = match (&mut frame, &self.inner) {
            (RawFrame::Video(f), EncoderType::Video(e)) => {
                let f = f.get_mut();
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
                    // Copy over PTS from old frame.
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
