use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    pin::Pin,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use ez_ffmpeg::{
    FfmpegContext, FfmpegScheduler, Input, Output, core::scheduler::ffmpeg_scheduler::Running,
};
use futures::{Sink, Stream};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub enum PipeInput {
    Network(String),
}

pub enum PipeOutput {
    Network(String),
    Raw(Arc<RawSinkSource>),
}

pub(crate) struct PipeConfig {
    input: PipeInput,
    outputs: Vec<PipeOutput>,
}

impl PipeConfig {
    pub fn new(input: PipeInput, outputs: Vec<PipeOutput>) -> Self {
        Self { input, outputs }
    }
}

pub(crate) struct Pipe {
    id: String,
    config: PipeConfig,
    cancel: CancellationToken,
    is_started: AtomicBool,
}

impl Pipe {
    pub fn new(id: &str, config: PipeConfig) -> Self {
        let cancel = CancellationToken::new();
        let is_started = AtomicBool::new(false);
        Self {
            id: id.to_string(),
            config,
            cancel,
            is_started,
        }
    }

    pub async fn start(&self) {
        if self.is_started.load(Ordering::Relaxed) {
            return;
        }

        let mut scheduler = None::<FfmpegScheduler<Running>>;
        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    if let Some(scheduler) = scheduler.take() {
                        scheduler.abort();
                    }
                    self.is_started.store(false, Ordering::Relaxed);
                    break;
                },
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    if let Ok(Some(result)) = self.start_inner() {
                        scheduler = Some(result);
                    }
                },
            }
        }
    }

    fn start_inner(&self) -> anyhow::Result<Option<FfmpegScheduler<Running>>> {
        if self.is_started.load(Ordering::Relaxed) {
            return Ok(None);
        }

        let input = &self.config.input;
        let input: Input = match &input {
            PipeInput::Network(url) => Input::new(url.to_string()).into(),
        };

        let builder = FfmpegContext::builder().input(input);
        let mut outputs: Vec<Output> = Vec::new();
        for o in &self.config.outputs {
            match o {
                PipeOutput::Network(url) => {
                    outputs.push(Output::new(url.to_string()).set_format("rtsp"))
                }
                PipeOutput::Raw(source) => {
                    let source_clone = source.clone();
                    outputs.push(
                        Output::new_by_write_callback({
                            move |buf| {
                                let _ = source_clone.writer.try_send(buf.to_vec());
                                buf.len() as i32
                            }
                        })
                        .set_format("rawvideo")
                        .into(),
                    )
                }
            }
        }

        if outputs.is_empty() {
            self.is_started.store(false, Ordering::Relaxed);
            return Err(anyhow::anyhow!("No outputs"));
        }

        let context = builder.outputs(outputs).build()?;
        let result = context.start()?;

        self.is_started.store(true, Ordering::Relaxed);
        Ok(Some(result))
    }
}

static PIPE_INSTANCES: LazyLock<RwLock<HashMap<String, Arc<Pipe>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub(crate) async fn new_pipe(id: &str, config: PipeConfig) -> Arc<Pipe> {
    let pipe = Arc::new(Pipe::new(id, config));

    let id_clone = id.to_string();
    PIPE_INSTANCES.write().await.insert(id_clone, pipe.clone());
    pipe
}

pub(crate) async fn get_pipe(id: &str) -> Option<Arc<Pipe>> {
    let id = id.to_string();
    PIPE_INSTANCES.read().await.get(&id).cloned()
}

pub struct VideoRawFrame {
    data: Bytes,
}

impl VideoRawFrame {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data: Bytes::from(data),
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

impl Display for VideoRawFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "VideoRawFrame {{ data: {} }}", self.data.len())
    }
}

// --- RawSinkSource: Sink<Vec<u8>> + Stream<VideoRawFrame> ---

use std::sync::Mutex;

pub struct RawSinkSource {
    pub writer: tokio::sync::mpsc::Sender<Vec<u8>>,
    inner: Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>,
}

impl RawSinkSource {
    pub fn new() -> Self {
        Self::with_capacity(32)
    }

    pub fn with_capacity(buffer_size: usize) -> Self {
        let (writer, receiver) = tokio::sync::mpsc::channel(buffer_size);
        Self {
            writer,
            inner: Mutex::new(receiver),
        }
    }

    pub fn stream(&self) -> RawFrameStream<'_> {
        RawFrameStream { source: self }
    }
}

impl Default for RawSinkSource {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RawFrameStream<'a> {
    source: &'a RawSinkSource,
}

impl Stream for RawFrameStream<'_> {
    type Item = VideoRawFrame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut guard = self.source.inner.lock().unwrap();
        guard.poll_recv(cx).map(|opt| opt.map(VideoRawFrame::new))
    }
}

impl Stream for RawSinkSource {
    type Item = VideoRawFrame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut guard = self.get_mut().inner.lock().unwrap();
        guard.poll_recv(cx).map(|opt| opt.map(VideoRawFrame::new))
    }
}

/// Wrapper to use `Arc<RawSinkSource>` as Stream (orphan rule workaround).
pub struct RawSinkSourceStream(pub Arc<RawSinkSource>);

impl Stream for RawSinkSourceStream {
    type Item = VideoRawFrame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let source = &self.0;
        let mut guard = source.inner.lock().unwrap();
        guard.poll_recv(cx).map(|opt| opt.map(VideoRawFrame::new))
    }
}

impl RawSinkSource {
    /// Returns a stream that yields VideoRawFrame. Use this when you have `Arc<RawSinkSource>`.
    pub fn as_stream(this: Arc<Self>) -> RawSinkSourceStream {
        RawSinkSourceStream(this)
    }
}

impl Sink<Vec<u8>> for RawSinkSource {
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.get_mut().writer.capacity() > 0 {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn start_send(self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        self.get_mut()
            .writer
            .try_send(item)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "channel closed"))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
