use ffmpeg_next::{Dictionary, Frame, Rational, format, packet::Ref};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = AvInput::new("scripts/test.mp4", None).unwrap();
    let task = AvInputTask::new();
    let mut receiver = task.subscribe();
    tokio::spawn(async move {
        while let Ok(packet) = receiver.recv().await {
            println!(
                "packet: index: {}, pts: {:?}, dts: {:?}, data len: {}",
                packet.index(),
                packet.pts(),
                packet.dts(),
                packet.size()
            );
        }
    });
    task.start(input).await;

    // check decoder
    // let decoder = Decoder::new(0);
    // let mut decoder_receiver = task.subscribe();
    // tokio::spawn(async move {
    //     while let Ok(packet) = decoder_receiver.recv().await {
    //         decoder.decode(&packet);
    //     }
    // });

    let cancel = task.get_cancel();
    tokio::select! {
        _ = cancel.cancelled() => {
            log::info!("input task cancelled");
        }
    }

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
                    log::info!("read packet task finished");
                    cancel_clone.cancel();
                }
                _ = cancel_clone.cancelled() => {
                    log::info!("read packet task cancelled");
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

struct AvInput {
    inner: format::context::Input,
}

impl AvInput {
    pub fn new(url: &str, options: Option<Dictionary>) -> anyhow::Result<Self> {
        let input = match options {
            Some(options) => format::input_with_dictionary(url, options),
            None => format::input(url),
        }?;
        Ok(Self { inner: input })
    }

    pub fn video_stream(&self) -> Option<format::stream::Stream> {
        self.inner.streams().best(ffmpeg_next::media::Type::Video)
    }

    pub fn audio_stream(&self) -> Option<format::stream::Stream> {
        self.inner.streams().best(ffmpeg_next::media::Type::Audio)
    }

    pub fn read_packet(&mut self) -> anyhow::Result<Option<RawPacket>> {
        loop {
            match self.inner.packets().next() {
                Some((_, packet)) => {
                    return Ok(Some(packet.into()));
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

impl From<ffmpeg_next::codec::packet::Packet> for RawPacket {
    fn from(packet: ffmpeg_next::codec::packet::Packet) -> Self {
        Self { packet: packet }
    }
}

impl Into<ffmpeg_next::codec::packet::Packet> for RawPacket {
    fn into(self) -> ffmpeg_next::codec::packet::Packet {
        self.packet
    }
}

struct Decoder {
    reader_stream_index: usize,
    video_decoder: Option<ffmpeg_next::codec::decoder::Video>,
}

impl Decoder {
    pub fn new(reader_stream_index: usize) -> Self {
        let mut decoder_ctx = ffmpeg_next::codec::Context::new();
        Self::set_decoder_context_time_base(&mut decoder_ctx, Rational::new(1, 90000));

        let decoder = decoder_ctx.decoder();
        let video_decoder = decoder.video().unwrap();
        Self {
            reader_stream_index,
            video_decoder: Some(video_decoder),
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

    pub fn decode(&mut self, packet: &RawPacket) -> anyhow::Result<()> {
        if let Some(video_decoder) = &mut self.video_decoder {
            video_decoder.send_packet(&packet.packet)?;
            let mut frame = unsafe { Frame::empty() };
            video_decoder.receive_frame(&mut frame)?;
            return Ok(());
        }

        Err(anyhow::anyhow!("no video decoder"))
    }
}
