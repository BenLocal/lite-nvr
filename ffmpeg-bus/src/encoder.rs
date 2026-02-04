use std::time::Duration;

use ffmpeg_next::{Dictionary, Rational, picture};
use tokio_util::sync::CancellationToken;

use crate::{frame::RawFrame, packet::RawPacket, stream::AvStream};

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

pub struct Encoder {
    stream: AvStream,
    inner: EncoderType,
    encoder_time_base: Rational,
    interleaved: bool,
    frame_index: i64,
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
        encoder.set_frame_rate(Some((30, 1)));
        encoder.set_time_base(ffmpeg_next::util::mathematics::rescale::TIME_BASE);

        let encoder = encoder.open_with(options.unwrap_or_default())?;
        let encoder_time_base: Rational = unsafe { (*encoder.0.as_ptr()).time_base.into() };
        Ok(Self {
            stream: stream.clone(),
            inner: EncoderType::Video(encoder),
            encoder_time_base: encoder_time_base,
            interleaved: false,
            frame_index: 0,
        })
    }

    pub fn send_frame(&mut self, frame: RawFrame) -> anyhow::Result<()> {
        self.inner.send_frame(frame, self.frame_index)?;
        self.frame_index += 1;
        Ok(())
    }

    pub fn encoder_receive_packet(&mut self) -> anyhow::Result<Option<RawPacket>> {
        self.inner.encoder_receive_packet(self.encoder_time_base)
    }
}

pub struct EncoderTask {
    cancel: CancellationToken,
    raw_chan: tokio::sync::broadcast::Sender<RawPacket>,
}

impl EncoderTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        let (sender, _) = tokio::sync::broadcast::channel(1024);

        Self {
            cancel,
            raw_chan: sender,
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RawPacket> {
        self.raw_chan.subscribe()
    }

    pub async fn start(
        &self,
        encoder: Encoder,
        mut encoder_receiver: tokio::sync::broadcast::Receiver<RawFrame>,
    ) {
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();

        tokio::spawn(async move {
            let (tx, rx) = std::sync::mpsc::channel::<RawFrame>();
            let handle =
                tokio::task::spawn_blocking(move || Self::encoder_loop(encoder, rx, sender_clone));
            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        break;
                    }
                    Ok(frame) = encoder_receiver.recv() => {
                        let _ = tx.send(frame);
                    }
                }
            }
            let _ = handle.await;
        });
    }

    fn encoder_loop(
        mut encoder: Encoder,
        rx: std::sync::mpsc::Receiver<RawFrame>,
        out: tokio::sync::broadcast::Sender<RawPacket>,
    ) {
        loop {
            match rx.recv_timeout(Duration::from_millis(1)) {
                Ok(frame) => {
                    if let RawFrame::Video(frame) = &frame {
                        println!(
                            "send video decode frame: width: {}, height: {}, format: {:?}, pts: {:?}",
                            frame.width(),
                            frame.height(),
                            frame.format(),
                            frame.pts()
                        );
                    }
                    if let Err(e) = encoder.send_frame(frame) {
                        log::error!("send packet error: {}", e);
                        continue;
                    }

                    loop {
                        match encoder.encoder_receive_packet() {
                            Ok(Some(packet)) => {
                                let _ = out.send(packet);
                            }
                            Ok(None) => break,
                            Err(e) => {
                                log::error!("receive packet error: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("receive frame error: {}", e);
                }
            }
        }
    }
}
