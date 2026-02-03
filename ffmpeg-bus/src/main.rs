use std::collections::HashMap;

use ffmpeg_next::{Dictionary, Rational, codec::Parameters, format, frame::Video};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = AvInput::new("scripts/test.mp4", None).unwrap();
    let streams = input.streams();
    for (index, stream) in streams.iter() {
        println!("stream: index: {}, id: {:?}", index, stream.parameters.id());
    }
    let task = AvInputTask::new();
    // let mut receiver = task.subscribe();
    // tokio::spawn(async move {
    //     while let Ok(packet) = receiver.recv().await {
    //         println!(
    //             "packet: index: {}, pts: {:?}, dts: {:?}, data len: {}",
    //             packet.index(),
    //             packet.pts(),
    //             packet.dts(),
    //             packet.size()
    //         );
    //     }
    // });

    //  decoder
    let mut decoder = Decoder::new(streams.get(&0).unwrap())?;
    let mut decoder_receiver = task.subscribe();
    tokio::spawn(async move {
        while let Ok(packet) = decoder_receiver.recv().await {
            if packet.index() != decoder.stream_index() {
                continue;
            }
            println!(
                "send packet: index: {}, pts: {:?}, dts: {:?}, data len: {}",
                packet.index(),
                packet.pts(),
                packet.dts(),
                packet.size()
            );
            if let Err(e) = decoder.send_packet(packet) {
                log::error!("send packet error: {}", e);
                continue;
            }

            'outer: loop {
                match decoder.receive_frame() {
                    Ok(Some(frame)) => {
                        println!(
                            "frame: width: {}, height: {}, format: {:?}",
                            frame.width(),
                            frame.height(),
                            frame.format()
                        );
                    }
                    Ok(None) => break 'outer,
                    Err(e) => {
                        log::error!("receive frame error: {}", e);
                        break 'outer;
                    }
                }
            }
        }
    });

    task.start(input).await;
    tokio::signal::ctrl_c().await?;
    println!("ctrl+c received");
    task.get_cancel().cancel();

    Ok(())
}

struct AvInputTask {
    cancel: CancellationToken,
    raw_chan: tokio::sync::broadcast::Sender<RawPacket>,
}

impl AvInputTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        let (sender, _) = tokio::sync::broadcast::channel(1024);

        Self {
            cancel,
            raw_chan: sender,
        }
    }

    pub async fn start(&self, mut input: AvInput) {
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();
        tokio::spawn(async move {
            let cancel_inner = cancel_clone.clone();
            let handle = tokio::task::spawn_blocking(move || {
                loop {
                    if cancel_inner.is_cancelled() {
                        break;
                    }

                    match input.read_packet() {
                        Ok(Some(packet)) => {
                            // Attempt to send, ignore send error (receiver dropped)
                            let _ = sender_clone.send(packet.into());
                        }
                        Ok(None) => {
                            // End of stream, break the loop
                            break;
                        }
                        Err(e) => {
                            log::error!("read packet error: {}", e);
                            break;
                        }
                    }
                }
            });

            tokio::select! {
                _ = handle => {
                    println!("read packet task finished");
                    cancel_clone.cancel();
                }
                _ = cancel_clone.cancelled() => {
                    println!("read packet task cancelled");
                }
            }
        });
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RawPacket> {
        self.raw_chan.subscribe()
    }

    pub fn get_cancel(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

struct AvStream {
    index: usize,
    parameters: Parameters,
    time_base: Rational,
}

impl From<format::stream::Stream<'_>> for AvStream {
    fn from(stream: format::stream::Stream<'_>) -> Self {
        Self {
            index: stream.index(),
            parameters: stream.parameters(),
            time_base: stream.time_base(),
        }
    }
}

impl Clone for AvStream {
    fn clone(&self) -> Self {
        Self {
            index: self.index,
            parameters: self.parameters.clone(),
            time_base: self.time_base,
        }
    }
}

struct AvInput {
    inner: format::context::Input,
    streams: HashMap<usize, AvStream>,
}

impl AvInput {
    pub fn new(url: &str, options: Option<Dictionary>) -> anyhow::Result<Self> {
        let input = match options {
            Some(options) => format::input_with_dictionary(url, options),
            None => format::input(url),
        }?;

        let mut streams = HashMap::new();
        for stream in input.streams() {
            streams.insert(stream.index(), AvStream::from(stream));
        }

        Ok(Self {
            inner: input,
            streams,
        })
    }

    pub fn streams(&self) -> HashMap<usize, AvStream> {
        self.streams.clone()
    }

    pub fn read_packet(&mut self) -> anyhow::Result<Option<RawPacket>> {
        loop {
            match self.inner.packets().next() {
                Some((stream, packet)) => {
                    return Ok(Some((packet, stream.time_base()).into()));
                }
                None => {
                    return Err(anyhow::anyhow!("read eof"));
                }
            }
        }
    }
}

#[derive(Clone)]
struct RawPacket {
    packet: ffmpeg_next::codec::packet::Packet,
    time_base: Rational,
}

impl RawPacket {
    pub fn pts(&self) -> Option<i64> {
        self.packet.pts()
    }

    pub fn dts(&self) -> Option<i64> {
        self.packet.dts()
    }

    pub fn size(&self) -> usize {
        self.packet.size()
    }

    pub fn index(&self) -> usize {
        self.packet.stream()
    }
}

impl From<(ffmpeg_next::codec::packet::Packet, Rational)> for RawPacket {
    fn from((packet, time_base): (ffmpeg_next::codec::packet::Packet, Rational)) -> Self {
        Self {
            packet: packet,
            time_base: time_base,
        }
    }
}

impl Into<ffmpeg_next::codec::packet::Packet> for RawPacket {
    fn into(self) -> ffmpeg_next::codec::packet::Packet {
        self.packet
    }
}

#[derive(Clone)]
struct RawVideoFrame {
    frame: ffmpeg_next::util::frame::Video,
}

impl From<ffmpeg_next::util::frame::Video> for RawVideoFrame {
    fn from(frame: ffmpeg_next::util::frame::Video) -> Self {
        Self { frame: frame }
    }
}

impl RawVideoFrame {
    pub fn width(&self) -> u32 {
        self.frame.width()
    }

    pub fn height(&self) -> u32 {
        self.frame.height()
    }

    pub fn format(&self) -> format::Pixel {
        self.frame.format()
    }
}

struct Decoder {
    stream: AvStream,
    video_decoder: Option<ffmpeg_next::codec::decoder::Video>,
    decoder_time_base: Rational,
}

impl Decoder {
    pub fn new(stream: &AvStream) -> anyhow::Result<Self> {
        let mut decoder_ctx = ffmpeg_next::codec::Context::new();
        set_decoder_context_time_base(&mut decoder_ctx, stream.time_base);
        decoder_ctx.set_parameters(stream.parameters.clone())?;

        let video_decoder = decoder_ctx.decoder().video()?;
        let decoder_time_base = video_decoder.time_base();

        if video_decoder.format() == format::Pixel::None
            || video_decoder.width() == 0
            || video_decoder.height() == 0
        {
            return Err(anyhow::anyhow!("missing codec parameters"));
        }

        Ok(Self {
            stream: stream.clone(),
            video_decoder: Some(video_decoder),
            decoder_time_base,
        })
    }

    pub fn send_packet(&mut self, packet: RawPacket) -> anyhow::Result<()> {
        if let Some(video_decoder) = &mut self.video_decoder {
            let time_base = packet.time_base;
            let mut packet = packet.packet;
            packet.rescale_ts(time_base, self.decoder_time_base);

            video_decoder.send_packet(&packet)?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("no video decoder"))
        }
    }

    pub fn receive_frame(&mut self) -> anyhow::Result<Option<RawVideoFrame>> {
        if let Some(video_decoder) = &mut self.video_decoder {
            let mut frame = Video::empty();
            match video_decoder.receive_frame(&mut frame) {
                Ok(()) => Ok(Some(RawVideoFrame::from(frame))),
                Err(ffmpeg_next::Error::Eof) => Err(anyhow::anyhow!("read eof")),
                Err(ffmpeg_next::Error::Other { errno })
                    if errno == ffmpeg_next::util::error::EAGAIN =>
                {
                    Ok(None)
                }
                Err(err) => Err(err.into()),
            }
        } else {
            Err(anyhow::anyhow!("no video decoder"))
        }
    }

    pub fn stream_index(&self) -> usize {
        self.stream.index
    }
}

fn set_decoder_context_time_base(
    decoder_context: &mut ffmpeg_next::codec::Context,
    time_base: Rational,
) {
    unsafe {
        (*decoder_context.as_mut_ptr()).time_base = time_base.into();
    }
}
