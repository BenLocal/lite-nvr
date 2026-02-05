use std::{collections::HashMap, hash::Hasher, pin::Pin, sync::Arc};

use futures::{Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use crate::{
    decoder::{Decoder, DecoderTask},
    frame::{RawFrameCmd, VideoFrame},
    input::{AvInput, AvInputTask},
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
                    return Err(anyhow::anyhow!("output already exists"));
                }

                // try to start input task
                if state.input_task.is_none() && state.input_config.is_some() {
                    Self::start_input_task(state).await?;
                }

                let input_stream = state
                    .input_stream_map
                    .iter()
                    .find(|s| s.is_video())
                    .ok_or(anyhow::anyhow!("stream not found"))?;
                let decoder_receiver = state
                    .input_task
                    .as_ref()
                    .ok_or(anyhow::anyhow!("input task not found"))?
                    .subscribe();
                let decoder = Decoder::new(input_stream)?;
                let decoder_task = DecoderTask::new();
                decoder_task.start(decoder, decoder_receiver).await;

                let stream = BroadcastStream::new(decoder_task.subscribe()).map(|cmd| match cmd {
                    Ok(cmd) => match cmd {
                        RawFrameCmd::Data(frame) => Some(VideoFrame::try_from(frame).unwrap()),
                        RawFrameCmd::EOF => None,
                    },
                    Err(e) => {
                        log::error!("decoder task error: {:#?}", e);
                        None
                    }
                });
                state.output_config.insert(id.clone(), output);
                result
                    .send(Ok(Box::pin(stream)))
                    .map_err(|_| anyhow::anyhow!("send result error: receiver dropped"))?;
            }
        }

        Ok(())
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
            state.input_stream_map.push(stream.clone());
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
    input_stream_map: Vec<AvStream>,
    decoder_task: HashMap<usize, DecoderTask>,
}

impl BusState {
    fn new() -> Self {
        Self {
            input_config: None,
            output_config: HashMap::new(),
            input_task: None,
            input_stream_map: Vec::new(),
            decoder_task: HashMap::new(),
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

pub struct OutputConfig {
    pub id: String,
    pub dest: OutputDest,
    pub encode: Option<EncodeConfig>,
}

pub enum OutputDest {
    Net { url: String },
    File { path: String },
    Raw,
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
