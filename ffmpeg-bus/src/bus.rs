use std::{
    collections::HashMap,
    hash::Hasher,
    sync::{Arc, atomic::AtomicBool},
};

use tokio_util::sync::CancellationToken;

use crate::{
    input::{AvInput, AvInputTask},
    media::stream::RawSinkSource,
    output::AvOutput,
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
                let res = if state.input.is_none() {
                    match input {
                        InputConfig::Net { url, .. } => {
                            let input = AvInput::new(&url, None)?;
                            state.input = Some(input);
                        }
                        InputConfig::File { path } => {
                            let input = AvInput::new(&path, None)?;
                            state.input = Some(input);
                        }
                        _ => {}
                    };

                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Input already exists"))
                };
                result
                    .send(res)
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::RemoveInput { result } => {
                for (_, input_task) in state.input_tasks.drain() {
                    input_task.stop();
                }
                if let Some(input) = state.input.take() {
                    drop(input);
                }
                result
                    .send(Ok(()))
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::AddOutput { output, result } => {
                if let Some(input) = state.input.as_ref() {
                    let streams = &input.streams();
                    let current_stream = streams.iter().find(|(_, stream)| stream.index() == 0);
                    // if let Some((index, stream)) = current_stream {
                    //     if !state.input_tasks.contains_key(index) {
                    //         let input_task = AvInputTask::new();
                    //         state.input_tasks.insert(*index, input_task);
                    //         input_task.start(state.input);
                    //     }
                    //     let input_task = state.input_tasks.get_mut(index).unwrap();
                    //     input_task.start(stream);
                    // }
                }
            }
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

    pub async fn add_output(&self, output: OutputConfig) -> anyhow::Result<()> {
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
    input: Option<AvInput>,
    /// key -> input stream index
    input_tasks: HashMap<usize, AvInputTask>,
}

impl BusState {
    fn new() -> Self {
        Self {
            input: None,
            input_tasks: HashMap::new(),
        }
    }
}

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
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
}

pub enum InputConfig {
    Net { url: String },
    File { path: String },
    Raw { sink: Arc<RawSinkSource> },
}

pub struct OutputConfig {
    id: String,
    dest: OutputDest,
    encode: Option<EncodeConfig>,
}

pub enum OutputDest {
    Net { url: String },
    File { path: String },
    Raw { sink: Arc<RawSinkSource> },
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
