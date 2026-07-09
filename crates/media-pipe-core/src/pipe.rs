use std::{
    backtrace::Backtrace,
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use ffmpeg_bus::bus::{Bus as FbBus, VideoRawFrameStream};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::{
    stream::RawSinkSource,
    types::{EncodeConfig, InputConfig, OutputConfig, OutputDest, PipeConfig, VideoRawFrame},
};

/// Pipeline: media processing using ffmpeg-bus
pub struct Pipe {
    config: PipeConfig,
    cancel: CancellationToken,
    started: AtomicBool,
    /// The live ffmpeg-bus handle while the pipe is running (set in `start`,
    /// cleared on teardown). Lets consumers such as ASR subscribe to the pipe's
    /// decoded audio without owning its internals.
    bus: Mutex<Option<Arc<FbBus>>>,
}

impl Pipe {
    pub fn new(config: PipeConfig) -> Self {
        Self {
            config,
            cancel: CancellationToken::new(),
            started: AtomicBool::new(false),
            bus: Mutex::new(None),
        }
    }

    /// Subscribe to this pipe's decoded-audio broadcast (for ASR). Errors if the
    /// pipe is not currently started.
    pub async fn subscribe_audio(&self) -> anyhow::Result<ffmpeg_bus::frame::RawFrameReceiver> {
        let bus = self
            .bus
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("pipe not started"))?;
        bus.subscribe_audio().await
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Check if the pipeline has been started
    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::Relaxed)
    }

    /// Check if the pipeline has been cancelled
    #[allow(dead_code)]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Start the pipeline. `input_options` are passed straight to the demuxer
    /// (e.g. `rtsp_transport=tcp` for RTSP); the caller decides transport policy
    /// so the core stays input-agnostic.
    pub async fn start(&self, input_options: Option<HashMap<String, String>>) {
        if self.started.swap(true, Ordering::Relaxed) {
            log::warn!("Pipe already started");
            return;
        }

        let log_input = match &self.config.input {
            InputConfig::Network { url } => format!("net://{}", url),
            InputConfig::File { path } => format!("file://{}", path),
            InputConfig::Device { display, format } => format!("device://{} ({})", display, format),
        };

        log::info!("Pipe: starting with input {}", log_input);

        let bus = Arc::new(FbBus::new("pipe"));
        // Publish the handle so consumers (ASR) can subscribe while we run.
        *self.bus.lock().unwrap() = Some(Arc::clone(&bus));
        let cancel = self.cancel.clone();

        // Map and add input
        let fb_input = self.config.input.clone().into();
        if let Err(e) = bus.add_input(fb_input, input_options).await {
            log::error!(
                "Pipe: add_input failed: {:#}\nbacktrace:\n{}",
                e,
                Backtrace::capture()
            );
            self.started.store(false, Ordering::Relaxed);
            return;
        }

        // First pass: register all outputs with the bus; collect successes. An
        // output may fail (e.g. an audio output when the input has no audio); we
        // notify a Demuxed sink so it can drop the missing sibling from any
        // coordination it does across video + audio.
        let mut accepted: Vec<(
            usize,
            ffmpeg_bus::stream::AvStream,
            VideoRawFrameStream,
            OutputConfig,
        )> = Vec::new();
        for (i, output_config) in self.config.outputs.iter().enumerate() {
            let id = format!("out_{}", i);
            let fb_output = match output_config.clone().into() {
                Some(o) => o,
                None => {
                    log::warn!(
                        "Pipe: skip unsupported output {:?}",
                        dest_name(&output_config.dest)
                    );
                    continue;
                }
            };
            match bus.add_output(fb_output).await {
                Ok((av, stream)) => {
                    accepted.push((i, av, stream, output_config.clone()));
                }
                Err(e) => {
                    log::warn!("Pipe: add_output {} failed: {:#}", id, e);
                    if let OutputDest::Demuxed { sink } = &output_config.dest {
                        sink.on_rejected();
                    }
                }
            }
        }

        // Second pass: spawn forwarder tasks into a JoinSet so the wait below
        // can observe the first one ending, then drain the rest on shutdown.
        let mut outputs = tokio::task::JoinSet::new();
        for (_, av, stream, output_config) in accepted {
            match &output_config.dest {
                OutputDest::RawFrame { sink } | OutputDest::RawPacket { sink } => {
                    let sink = Arc::clone(sink);
                    outputs.spawn(async move {
                        forward_frame_stream_to_sink(stream, sink).await;
                    });
                }
                OutputDest::Demuxed { sink } => {
                    let handle = sink.start(av, stream);
                    outputs.spawn(async move {
                        let _ = handle.await;
                    });
                }
                OutputDest::Network { .. } => {}
            }
        }

        if outputs.is_empty() && !self.config.outputs.is_empty() {
            log::warn!("Pipe: no output task running");
        }

        // Wait for cancellation — or for an output task to end. Forwarders only
        // end when the input side is done (EOF, read error, sink gone), so the
        // first completion means the session is dead and start() must unwind
        // instead of idling forever; that lets a supervisor observe stream
        // death and restart (e.g. re-resolving an expired live-stream URL).
        // Pipes whose outputs are all in-bus (Network) keep the cancel-only wait.
        if outputs.is_empty() {
            cancel.cancelled().await;
            log::info!("Pipe: cancelled");
        } else {
            tokio::select! {
                _ = cancel.cancelled() => {
                    log::info!("Pipe: cancelled");
                }
                _ = outputs.join_next() => {
                    log::info!("Pipe: output ended (input finished), stopping");
                }
            }
        }

        // Stop input and outputs: remove input first so the bus stops feeding streams
        if let Err(e) = bus.remove_input().await {
            log::warn!("Pipe: remove_input failed: {:#}", e);
        }
        bus.stop();
        // Unpublish before dropping the last handle; new subscribers now error.
        *self.bus.lock().unwrap() = None;
        while outputs.join_next().await.is_some() {}

        self.started.store(false, Ordering::Relaxed);
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Forwards ffmpeg-bus VideoFrame stream to a [`RawSinkSource`] (VideoRawFrame).
async fn forward_frame_stream_to_sink(
    mut stream: ffmpeg_bus::bus::VideoRawFrameStream,
    sink: Arc<RawSinkSource>,
) {
    while let Some(opt) = stream.next().await {
        if let Some(frame) = opt {
            let vf = VideoRawFrame::new(
                frame.data.to_vec(),
                frame.width,
                frame.height,
                frame.format,
                frame.pts,
                frame.dts,
                frame.is_key,
                frame.codec_id,
            );
            if sink.writer.try_send(vf).is_err() {
                break;
            }
        }
    }
}

/// Get destination name for logging (used by tests).
pub fn dest_name(dest: &OutputDest) -> String {
    match dest {
        OutputDest::Network { url, .. } => url.clone(),
        OutputDest::RawFrame { .. } => "RawFrame".to_string(),
        OutputDest::RawPacket { .. } => "RawPacket".to_string(),
        OutputDest::Demuxed { .. } => "Demuxed".to_string(),
    }
}

impl PipeConfig {
    #[allow(dead_code)]
    pub fn builder() -> PipeConfigBuilder {
        PipeConfigBuilder::default()
    }
}

#[allow(dead_code)]
#[derive(Default)]
pub struct PipeConfigBuilder {
    input: Option<InputConfig>,
    outputs: Vec<OutputConfig>,
}

#[allow(dead_code)]
impl PipeConfigBuilder {
    /// Set network input source
    pub fn input_url(mut self, url: impl Into<String>) -> Self {
        self.input = Some(InputConfig::Network { url: url.into() });
        self
    }

    /// Set file input source
    pub fn input_file(mut self, path: impl Into<String>) -> Self {
        self.input = Some(InputConfig::File { path: path.into() });
        self
    }

    /// Add RTSP output
    /// if encode is None, the output will be remuxed
    /// if encode is Some, the output will be encoded
    pub fn add_rtsp_output(mut self, url: impl Into<String>, encode: Option<EncodeConfig>) -> Self {
        self.outputs.push(OutputConfig::new(
            OutputDest::Network {
                url: url.into(),
                format: "rtsp".to_string(),
            },
            encode,
        ));

        self
    }

    /// Add direct remux output (no re-encoding)
    pub fn add_remux_output(mut self, url: impl Into<String>, format: impl Into<String>) -> Self {
        self.outputs.push(OutputConfig::new(
            OutputDest::Network {
                url: url.into(),
                format: format.into(),
            },
            None,
        ));
        self
    }

    /// Add raw frame output
    pub fn add_raw_frame_output(mut self, sink: Arc<RawSinkSource>) -> Self {
        self.outputs
            .push(OutputConfig::new(OutputDest::RawFrame { sink }, None));
        self
    }

    /// Add encoded packet output
    pub fn add_raw_packet_output(mut self, sink: Arc<RawSinkSource>, encode: EncodeConfig) -> Self {
        self.outputs.push(OutputConfig::new(
            OutputDest::RawPacket { sink },
            Some(encode),
        ));
        self
    }

    pub fn build(self) -> PipeConfig {
        PipeConfig {
            input: self.input.expect("input is required"),
            outputs: self.outputs,
        }
    }
}

#[cfg(test)]
#[path = "pipe_test.rs"]
mod pipe_test;
