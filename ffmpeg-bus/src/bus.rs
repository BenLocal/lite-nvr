use std::{collections::HashMap, hash::Hasher, pin::Pin, sync::Arc};

use futures::{Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use ffmpeg_next::Dictionary;

use crate::{
    decoder::{Decoder, DecoderTask},
    encoder::{Encoder, EncoderTask, Settings},
    frame::{RawFrameCmd, VideoFrame},
    input::{AvInput, AvInputTask},
    output::{AvOutput, AvOutputStream},
    packet::RawPacketCmd,
    sink::RawSinkSource,
    stream::AvStream,
};

pub struct Bus {
    id: String,
    cancel: CancellationToken,
    tx: tokio::sync::mpsc::Sender<BusCommand>,
}

impl Bus {
    pub fn new(id: &str) -> Self {
        let id = id.to_string();
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::channel(1024);

        let cancel_clone = cancel.clone();
        tokio::spawn(async move { Self::inner_loop(cancel_clone, rx).await });
        Self { id: id, cancel, tx }
    }

    async fn inner_loop(
        cancel: CancellationToken,
        mut rx: tokio::sync::mpsc::Receiver<BusCommand>,
    ) {
        let cancel_clone = cancel.clone();
        let mut state = BusState::new();
        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    break;
                },
                Some(cmd) = rx.recv() => {
                    if let Err(e) = Self::inner_command_handler(&mut state, cmd).await {
                        log::error!("inner_command_handler error: {:#?}", e);
                    }
                },
            }
        }
    }

    async fn inner_command_handler(state: &mut BusState, cmd: BusCommand) -> anyhow::Result<()> {
        match cmd {
            BusCommand::AddInput { input, result } => {
                result
                    .send(Self::add_input_internal(state, input).await)
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::RemoveInput { result } => {
                if let Some(input) = state.input_task.take() {
                    input.stop();
                    drop(input);
                }
                state.input_config = None;
                result
                    .send(Ok(()))
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::AddOutput { output, result } => {
                let id = &output.id;
                if state.output_config.contains_key(id) {
                    let _ = result.send(Err(anyhow::anyhow!("output already exists")));
                    return Err(anyhow::anyhow!("output already exists"));
                }

                // try to start input task
                if state.input_task.is_none() && state.input_config.is_some() {
                    if let Err(e) = Self::start_input_task(state).await {
                        let msg = format!("{:#}", e);
                        let _ = result.send(Err(anyhow::anyhow!("{}", msg)));
                        return Err(anyhow::anyhow!("{}", msg));
                    }
                }
                let input_stream = state
                    .input_streams
                    .iter()
                    .find(|s| match output.av_type {
                        OutputAvType::Video => s.is_video(),
                        OutputAvType::Audio => s.is_audio(),
                    })
                    .ok_or(anyhow::anyhow!("stream not found"))?;
                let input_stream_index = input_stream.index();
                let need_decoder = Self::try_decoder(&output)?;
                if need_decoder {
                    Self::start_decoder_task(state, input_stream_index).await?;
                }

                let need_encoder = Self::try_encoder(&output)?;
                if need_encoder {
                    Self::start_encoder_task(state, input_stream_index).await?;
                }

                let stream_result = match &output.dest {
                    OutputDest::Raw => {
                        Self::create_decoder_raw_output_stream(state, input_stream_index).await
                    }
                    OutputDest::File { path } => {
                        Self::create_mux_to_file(state, path, input_stream_index).await
                    }
                    OutputDest::Net { url, format } => {
                        Self::create_mux_to_net(state, url, format.as_deref(), input_stream_index)
                            .await
                    }
                    OutputDest::Mux { format } => {
                        Self::create_mux_output_stream(state, format, input_stream_index).await
                    }
                    OutputDest::Encoded => {
                        Self::create_encoded_output_stream(state, input_stream_index).await
                    }
                };

                match stream_result {
                    Ok(stream) => {
                        state.output_config.insert(id.clone(), output);
                        result
                            .send(Ok(stream))
                            .map_err(|_| anyhow::anyhow!("send result error: receiver dropped"))?;
                    }
                    Err(e) => {
                        let msg = format!("{:#}", e);
                        let _ = result.send(Err(anyhow::anyhow!("{}", msg)));
                        return Err(anyhow::anyhow!("{}", msg));
                    }
                }
            }
        }

        Ok(())
    }

    fn try_decoder(output: &OutputConfig) -> anyhow::Result<bool> {
        match &output.dest {
            OutputDest::Raw => Ok(true),
            OutputDest::File { .. } => Ok(false),
            OutputDest::Mux { .. } => Ok(false),
            OutputDest::Net { .. } => Ok(true),
            OutputDest::Encoded => Ok(true),
        }
    }

    fn try_encoder(output: &OutputConfig) -> anyhow::Result<bool> {
        if let OutputDest::Raw = output.dest {
            return Ok(false);
        }

        if let OutputDest::Encoded = output.dest {
            return Ok(true);
        }

        if output.encode.is_some() {
            return Ok(true);
        }

        Ok(false)
    }

    /// Mux to a real file path (seekable). Produces standard MP4 that any player can open.
    async fn create_mux_to_file(
        state: &mut BusState,
        path: &str,
        input_stream_index: usize,
    ) -> anyhow::Result<VideoRawFrameStream> {
        let mut input_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();

        let target_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("no matching stream in input"))?
            .clone();
        let target_stream_index = target_stream.index();
        let path_owned = path.to_string();

        let mut output = AvOutput::new(path, None, None)?;
        output.add_stream(&target_stream)?;

        tokio::spawn(async move {
            let mut output = output;
            while let Ok(cmd) = input_receiver.recv().await {
                match cmd {
                    RawPacketCmd::Data(packet) => {
                        if packet.index() == target_stream_index {
                            if let Err(e) = output.write_packet(target_stream_index, packet) {
                                log::error!("mux to file write_packet error: {:#?}", e);
                            }
                        }
                    }
                    RawPacketCmd::EOF => break,
                }
            }
            if let Err(e) = output.finish() {
                log::error!("mux to file finish error: {:#?}", e);
            }
            println!("mux to file finished: {}", path_owned);
        });

        Ok(Box::pin(futures::stream::empty::<Option<VideoFrame>>()))
    }

    /// Mux to a network URL (e.g. rtmp://, rtsp://). Remux only (input packets).
    /// format: e.g. Some("rtsp"), Some("flv"); None = let FFmpeg guess from URL.
    async fn create_mux_to_net(
        state: &mut BusState,
        url: &str,
        format: Option<&str>,
        input_stream_index: usize,
    ) -> anyhow::Result<VideoRawFrameStream> {
        let mut input_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();

        let target_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("no matching stream in input"))?
            .clone();
        let target_stream_index = target_stream.index();
        let url_owned = url.to_string();

        // RTSP output often needs rtsp_transport=tcp for avio_open2 to succeed
        let options = match format {
            Some("rtsp") => {
                let mut opts = Dictionary::new();
                opts.set("rtsp_transport", "tcp");
                Some(opts)
            }
            _ => None,
        };

        let mut output = AvOutput::new(url, format, options).map_err(|e| {
            anyhow::anyhow!(
                "create_mux_to_net AvOutput::new(url={:?}, format={:?}): {:?}",
                url,
                format,
                e
            )
        })?;
        output
            .add_stream(&target_stream)
            .map_err(|e| anyhow::anyhow!("create_mux_to_net add_stream: {:?}", e))?;

        tokio::spawn(async move {
            let mut output = output;
            while let Ok(cmd) = input_receiver.recv().await {
                match cmd {
                    RawPacketCmd::Data(packet) => {
                        if packet.index() == target_stream_index {
                            if let Err(e) = output.write_packet(target_stream_index, packet) {
                                log::error!("mux to net write_packet error: {:#?}", e);
                            }
                        }
                    }
                    RawPacketCmd::EOF => break,
                }
            }
            if let Err(e) = output.finish() {
                log::error!("mux to net finish error: {:#?}", e);
            }
            log::info!("mux to net finished: {}", url_owned);
        });

        Ok(Box::pin(futures::stream::empty::<Option<VideoFrame>>()))
    }

    async fn create_encoded_output_stream(
        state: &mut BusState,
        input_stream_index: usize,
    ) -> anyhow::Result<VideoRawFrameStream> {
        let encoder_receiver = state
            .encoder_tasks
            .get(&input_stream_index)
            .ok_or(anyhow::anyhow!("encoder task not found"))?
            .subscribe();

        let stream = BroadcastStream::new(encoder_receiver).filter_map(|r| async move {
            match r {
                Ok(RawPacketCmd::Data(packet)) => Some(Some(VideoFrame::from(packet))),
                Ok(RawPacketCmd::EOF) => Some(None),
                Err(_) => None,
            }
        });

        Ok(Box::pin(stream))
    }

    async fn create_mux_output_stream(
        state: &mut BusState,
        format: &str,
        input_stream_index: usize,
    ) -> anyhow::Result<VideoRawFrameStream> {
        let mut input_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();

        let target_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("no matching stream in input"))?;
        let target_stream_index = target_stream.index();
        let mut stream = AvOutputStream::new(format)?;
        stream.add_stream(&target_stream)?;
        let (writer, reader) = stream.into_split();

        tokio::spawn(async move {
            let mut writer = writer;
            while let Ok(cmd) = input_receiver.recv().await {
                match cmd {
                    RawPacketCmd::Data(packet) => {
                        if packet.index() == target_stream_index {
                            if let Err(e) = writer.write_packet(packet) {
                                log::error!("mux write_packet error: {:#?}", e);
                            }
                        }
                    }
                    RawPacketCmd::EOF => break,
                }
            }
            if let Err(e) = writer.finish() {
                log::error!("mux finish error: {:#?}", e);
            }
            println!("mux stream finished");
        });

        Ok(Box::pin(reader.map(|pkg| Some(VideoFrame::from(pkg)))))
    }

    async fn create_decoder_raw_output_stream(
        state: &mut BusState,
        stream_index: usize,
    ) -> anyhow::Result<VideoRawFrameStream> {
        let stream = BroadcastStream::new(
            state
                .decoder_tasks
                .get(&stream_index)
                .ok_or(anyhow::anyhow!("decoder task not found"))?
                .subscribe(),
        )
        .map(|cmd| match cmd {
            Ok(cmd) => match cmd {
                RawFrameCmd::Data(frame) => Some(VideoFrame::try_from(frame).unwrap()),
                RawFrameCmd::EOF => None,
            },
            Err(e) => {
                log::error!("decoder task error: {:#?}", e);
                None
            }
        });

        Ok(Box::pin(stream))
    }

    async fn add_input_internal(state: &mut BusState, input: InputConfig) -> anyhow::Result<()> {
        if state.input_config.is_some() {
            return Err(anyhow::anyhow!("input already exists"));
        } else {
            state.input_config = Some(input);
        }

        if !state.output_config.is_empty() && state.input_task.is_none() {
            Self::start_input_task(state).await?;
        }
        Ok(())
    }

    async fn start_encoder_task(
        state: &mut BusState,
        input_stream_index: usize,
    ) -> anyhow::Result<()> {
        let input_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?;
        if state.encoder_tasks.contains_key(&input_stream_index) {
            return Ok(());
        }
        let encoder_receiver = state
            .decoder_tasks
            .get(&input_stream_index)
            .ok_or(anyhow::anyhow!("decoder task not found"))?
            .subscribe();
        let encoder = Encoder::new(input_stream, Settings::default(), None)?;
        let encoder_task = EncoderTask::new();
        encoder_task.start(encoder, encoder_receiver).await;
        state.encoder_tasks.insert(input_stream_index, encoder_task);
        Ok(())
    }

    async fn start_decoder_task(
        state: &mut BusState,
        input_stream_index: usize,
    ) -> anyhow::Result<()> {
        let input_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?;
        if state.decoder_tasks.contains_key(&input_stream_index) {
            return Ok(());
        }
        let decoder_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();
        let decoder = Decoder::new(input_stream)?;
        let decoder_task = DecoderTask::new();
        decoder_task.start(decoder, decoder_receiver).await;
        state.decoder_tasks.insert(input_stream_index, decoder_task);

        Ok(())
    }

    async fn start_input_task(state: &mut BusState) -> anyhow::Result<()> {
        let input = match state.input_config.as_ref() {
            Some(InputConfig::Net { url }) => AvInput::new(url, None)?,
            Some(InputConfig::File { path }) => AvInput::new(path, None)?,
            Some(_) => unimplemented!(),
            None => return Err(anyhow::anyhow!("input config is not set")),
        };

        let streams = input.streams();
        println!("start add input streams: ");
        for (index, stream) in streams {
            println!(
                "stream index: {}, stream id: {:#?}, time_base: {:#?}",
                index,
                stream.parameters().id(),
                stream.time_base()
            );
            state.input_streams.push(stream.clone());
        }

        state.input_task = Some(AvInputTask::new());

        if let Some(task) = state.input_task.as_ref() {
            task.start(input).await;
        }

        Ok(())
    }

    pub async fn add_input(&self, input: InputConfig) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::AddInput { input, result: tx })
            .await?;
        rx.await?
    }

    pub async fn remove_input(&self) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(BusCommand::RemoveInput { result: tx }).await?;
        rx.await?
    }

    pub async fn add_output(&self, output: OutputConfig) -> anyhow::Result<VideoRawFrameStream> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::AddOutput { output, result: tx })
            .await?;
        rx.await?
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        self.stop();
    }
}

struct BusState {
    input_config: Option<InputConfig>,
    output_config: HashMap<String, OutputConfig>,
    input_task: Option<AvInputTask>,
    input_streams: Vec<AvStream>,
    decoder_tasks: HashMap<usize, DecoderTask>,
    encoder_tasks: HashMap<usize, EncoderTask>,
}

impl BusState {
    fn new() -> Self {
        Self {
            input_config: None,
            output_config: HashMap::new(),
            input_task: None,
            input_streams: Vec::new(),
            decoder_tasks: HashMap::new(),
            encoder_tasks: HashMap::new(),
        }
    }
}

pub type VideoRawFrameStream = Pin<Box<dyn Stream<Item = Option<VideoFrame>> + Send + Sync>>;

pub enum BusCommand {
    AddInput {
        input: InputConfig,
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    RemoveInput {
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    AddOutput {
        output: OutputConfig,
        result: tokio::sync::oneshot::Sender<anyhow::Result<VideoRawFrameStream>>,
    },
}

pub enum InputConfig {
    Net { url: String },
    File { path: String },
    Raw { sink: Arc<RawSinkSource> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputAvType {
    Video,
    Audio,
}

pub struct OutputConfig {
    pub id: String,
    pub dest: OutputDest,
    pub av_type: OutputAvType,
    pub encode: Option<EncodeConfig>,
}

impl OutputConfig {
    pub fn new(id: String, av_type: OutputAvType, dest: OutputDest) -> Self {
        Self {
            id,
            dest,
            av_type,
            encode: None,
        }
    }

    pub fn with_encode(mut self, encode: EncodeConfig) -> Self {
        self.encode = Some(encode);
        self
    }
}

pub enum OutputDest {
    ///! Mux to a network stream (no seekable), some times called live streaming
    ///! eg: rtmp://localhost:1935/live/stream
    ///! eg: rtsp://host:8554/path
    ///! format: e.g. "rtsp", "flv" (required for URL-only outputs; None = guess from URL)
    Net { url: String, format: Option<String> },
    /// Mux to a file (seekable). Produces standard MP4 that any player can open.
    File { path: String },
    /// Raw video frames (only support decode, no encoding)
    Raw,
    /// Mux to a stream (no seekable)
    Mux { format: String },
    /// Stream of encoded packets (e.g. for RawPacket sink). Requires encoder.
    Encoded,
}

#[derive(Clone, Debug)]
pub struct EncodeConfig {
    // "h264", "hevc", "rawvideo"
    pub codec: String,
    // None = keep original
    pub width: Option<u32>,
    // None = keep original
    pub height: Option<u32>,
    // bps
    pub bitrate: Option<u64>,
    // "ultrafast", "medium", etc.
    pub preset: Option<String>,
    // "yuv420p", "rgb24", etc.
    pub pixel_format: Option<String>,
}

impl Default for EncodeConfig {
    fn default() -> Self {
        Self {
            codec: "h264".to_string(),
            width: None,
            height: None,
            bitrate: None,
            preset: None,
            pixel_format: None,
        }
    }
}

impl PartialEq for EncodeConfig {
    fn eq(&self, other: &Self) -> bool {
        self.codec == other.codec
            && self.width == other.width
            && self.height == other.height
            && self.bitrate == other.bitrate
            && self.preset == other.preset
            && self.pixel_format == other.pixel_format
    }
}

impl Eq for EncodeConfig {}

impl std::hash::Hash for EncodeConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.codec.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.bitrate.hash(state);
        self.preset.hash(state);
        self.pixel_format.hash(state);
    }
}

#[cfg(test)]
#[path = "bus_test.rs"]
mod bus_test;
