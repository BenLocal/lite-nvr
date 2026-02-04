use std::time::Duration;

use ffmpeg_next::Rational;
use tokio_util::sync::CancellationToken;

use crate::{
    frame::{RawAudioFrame, RawFrame, RawVideoFrame},
    packet::RawPacket,
    stream::AvStream,
};

enum DecoderType {
    Video(ffmpeg_next::codec::decoder::Video),
    Audio(ffmpeg_next::codec::decoder::Audio),
}

impl DecoderType {
    pub fn send_packet(
        &mut self,
        mut packet: RawPacket,
        decoder_time_base: Rational,
    ) -> anyhow::Result<()> {
        let time_base = packet.time_base();
        let packet = packet.get_mut();
        packet.rescale_ts(time_base, decoder_time_base);
        match self {
            DecoderType::Video(video_decoder) => {
                video_decoder.send_packet(packet)?;
            }
            DecoderType::Audio(audio_decoder) => {
                audio_decoder.send_packet(packet)?;
            }
        }

        Ok(())
    }

    pub fn receive_frame(&mut self) -> anyhow::Result<Option<RawFrame>> {
        match self {
            DecoderType::Video(video_decoder) => {
                let mut frame = ffmpeg_next::frame::Video::empty();
                match video_decoder.receive_frame(&mut frame) {
                    Ok(()) => Ok(Some(RawFrame::Video(RawVideoFrame::from(frame)))),
                    Err(ffmpeg_next::Error::Eof) => Err(anyhow::anyhow!("read eof")),
                    Err(ffmpeg_next::Error::Other { errno })
                        if errno == ffmpeg_next::util::error::EAGAIN =>
                    {
                        Ok(None)
                    }
                    Err(err) => Err(err.into()),
                }
            }
            DecoderType::Audio(audio_decoder) => {
                let mut frame = ffmpeg_next::frame::Audio::empty();
                match audio_decoder.receive_frame(&mut frame) {
                    Ok(()) => Ok(Some(RawFrame::Audio(RawAudioFrame::from(frame)))),
                    Err(ffmpeg_next::Error::Eof) => Err(anyhow::anyhow!("read eof")),
                    Err(ffmpeg_next::Error::Other { errno })
                        if errno == ffmpeg_next::util::error::EAGAIN =>
                    {
                        Ok(None)
                    }
                    Err(err) => Err(err.into()),
                }
            }
        }
    }
}

pub struct Decoder {
    stream: AvStream,
    inner: DecoderType,
    decoder_time_base: Rational,
}

impl Decoder {
    pub fn new(stream: &AvStream) -> anyhow::Result<Self> {
        let mut decoder_ctx = ffmpeg_next::codec::Context::new();
        unsafe {
            (*decoder_ctx.as_mut_ptr()).time_base = stream.time_base().into();
        }
        decoder_ctx.set_parameters(stream.parameters().clone())?;

        let s = if stream.is_video() {
            let video_decoder = decoder_ctx.decoder().video()?;
            let decoder_time_base = video_decoder.time_base();

            if video_decoder.format() == ffmpeg_next::format::Pixel::None
                || video_decoder.width() == 0
                || video_decoder.height() == 0
            {
                return Err(anyhow::anyhow!("missing codec parameters"));
            }

            Self {
                stream: stream.clone(),
                inner: DecoderType::Video(video_decoder),
                decoder_time_base,
            }
        } else if stream.is_audio() {
            let audio_decoder = decoder_ctx.decoder().audio()?;
            let decoder_time_base = audio_decoder.time_base();
            Self {
                stream: stream.clone(),
                inner: DecoderType::Audio(audio_decoder),
                decoder_time_base,
            }
        } else {
            return Err(anyhow::anyhow!("unsupported stream type"));
        };

        Ok(s)
    }

    pub fn send_packet(&mut self, packet: RawPacket) -> anyhow::Result<()> {
        self.inner.send_packet(packet, self.decoder_time_base)
    }

    pub fn receive_frame(&mut self) -> anyhow::Result<Option<RawFrame>> {
        self.inner.receive_frame()
    }

    pub fn stream_index(&self) -> usize {
        self.stream.index()
    }
}

pub struct DecoderTask {
    cancel: CancellationToken,
    raw_chan: tokio::sync::broadcast::Sender<RawFrame>,
}

impl DecoderTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        let (sender, _) = tokio::sync::broadcast::channel(1024);

        Self {
            cancel,
            raw_chan: sender,
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RawFrame> {
        self.raw_chan.subscribe()
    }

    pub async fn start(
        &self,
        decoder: Decoder,
        mut decoder_receiver: tokio::sync::broadcast::Receiver<RawPacket>,
    ) {
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();
        tokio::spawn(async move {
            let (packet_tx, packet_rx) = std::sync::mpsc::channel::<RawPacket>();
            let current_stream_index = decoder.stream_index();
            let handle = tokio::task::spawn_blocking(move || {
                Self::decoder_loop(decoder, packet_rx, sender_clone)
            });
            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        break;
                    }
                    Ok(packet) = decoder_receiver.recv() => {
                        if packet.index() != current_stream_index {
                            continue;
                        }
                        let _ =  packet_tx.send(packet);
                    }
                }
            }
            let _ = handle.await;
        });
    }

    fn decoder_loop(
        mut decoder: Decoder,
        packet_rx: std::sync::mpsc::Receiver<RawPacket>,
        out_sender: tokio::sync::broadcast::Sender<RawFrame>,
    ) {
        loop {
            match packet_rx.recv_timeout(Duration::from_millis(1)) {
                Ok(packet) => {
                    if let Err(e) = decoder.send_packet(packet) {
                        log::error!("send packet error: {}", e);
                        continue;
                    }
                    'outer: loop {
                        match decoder.receive_frame() {
                            Ok(Some(RawFrame::Video(frame))) => {
                                let _ = out_sender.send(RawFrame::Video(frame));
                            }
                            Ok(Some(RawFrame::Audio(frame))) => {
                                let _ = out_sender.send(RawFrame::Audio(frame));
                            }
                            Ok(None) => break 'outer,
                            Err(e) => {
                                log::error!("receive frame error: {}", e);
                                break 'outer;
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("receive packet error: {}", e);
                }
            }
        }
    }
}
