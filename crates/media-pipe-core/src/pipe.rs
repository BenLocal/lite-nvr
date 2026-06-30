use std::{
    backtrace::Backtrace,
    collections::HashMap,
    sync::{
        Arc,
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
}

impl Pipe {
    pub fn new(config: PipeConfig) -> Self {
        Self {
            config,
            cancel: CancellationToken::new(),
            started: AtomicBool::new(false),
        }
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

    /// Start the pipeline
    pub async fn start(&self) {
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

        let bus = FbBus::new("pipe");
        let cancel = self.cancel.clone();

        // RTSP over UDP (FFmpeg's default) drops packets on lossy/jittery links,
        // which corrupts the H264 stream ("RTP: missed packets" -> decode errors).
        // Force TCP transport with a socket timeout for RTSP inputs.
        let input_options = match &self.config.input {
            InputConfig::Network { url } if url.starts_with("rtsp://") => Some(HashMap::from([
                ("rtsp_transport".to_string(), "tcp".to_string()),
                ("stimeout".to_string(), "5000000".to_string()),
            ])),
            _ => None,
        };

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

        // Second pass: spawn forwarder tasks.
        let mut join_handles = Vec::new();
        for (_, av, stream, output_config) in accepted {
            match &output_config.dest {
                OutputDest::RawFrame { sink } => {
                    let sink = Arc::clone(sink);
                    let handle = tokio::spawn(async move {
                        forward_frame_stream_to_sink(stream, sink).await;
                    });
                    join_handles.push(handle);
                }
                OutputDest::RawPacket { sink } => {
                    let sink = Arc::clone(sink);
                    let handle = tokio::spawn(async move {
                        forward_frame_stream_to_sink(stream, sink).await;
                    });
                    join_handles.push(handle);
                }
                OutputDest::Demuxed { sink } => {
                    join_handles.push(sink.start(av, stream));
                }
                OutputDest::Network { .. } => {}
            }
        }

        if join_handles.is_empty() && !self.config.outputs.is_empty() {
            log::warn!("Pipe: no output task running");
        }

        // Wait for cancellation
        tokio::select! {
            _ = cancel.cancelled() => {
                log::info!("Pipe: cancelled");
            }
        }

        // Stop input and outputs: remove input first so the bus stops feeding streams
        if let Err(e) = bus.remove_input().await {
            log::warn!("Pipe: remove_input failed: {:#}", e);
        }
        bus.stop();
        for h in join_handles {
            let _ = h.await;
        }

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
