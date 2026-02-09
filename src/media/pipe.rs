use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use ffmpeg_bus::bus::{
    Bus as FbBus, InputConfig as FbInputConfig, OutputAvType, OutputConfig as FbOutputConfig,
    OutputDest as FbOutputDest,
};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::media::{
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
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Start the pipeline
    pub async fn start(&self) {
        if self.started.swap(true, Ordering::Relaxed) {
            log::warn!("Pipe already started");
            return;
        }

        let input_url = match &self.config.input {
            InputConfig::Network { url } => url.clone(),
            InputConfig::File { path } => path.clone(),
        };

        log::info!("Pipe: starting with input {}", input_url);

        let bus = FbBus::new("pipe");
        let cancel = self.cancel.clone();

        // Map and add input
        let fb_input = to_fb_input(&self.config.input);
        if let Err(e) = bus.add_input(fb_input).await {
            log::error!("Pipe: add_input failed: {:#}", e);
            self.started.store(false, Ordering::Relaxed);
            return;
        }

        // Add each output and optionally forward stream to sink
        let mut join_handles = Vec::new();
        for (i, output_config) in self.config.outputs.iter().enumerate() {
            let id = format!("out_{}", i);
            let fb_output = match to_fb_output(&id, output_config) {
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
                Ok(stream) => {
                    // RawFrame or RawPacket: forward stream to sink
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
                        OutputDest::Network { .. } => {}
                    }
                }
                Err(e) => {
                    log::warn!("Pipe: add_output {} failed: {:#}", id, e);
                }
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

        bus.stop();
        for h in join_handles {
            let _ = h.await;
        }

        self.started.store(false, Ordering::Relaxed);
    }
}

fn to_fb_input(input: &InputConfig) -> FbInputConfig {
    match input {
        InputConfig::Network { url } => FbInputConfig::Net { url: url.clone() },
        InputConfig::File { path } => FbInputConfig::File { path: path.clone() },
    }
}

fn to_fb_output(id: &str, config: &OutputConfig) -> Option<FbOutputConfig> {
    let av_type = OutputAvType::Video; // pipe only uses video for now
    let dest = match &config.dest {
        OutputDest::Network { url, format } => FbOutputDest::Net {
            url: url.clone(),
            format: Some(format.clone()),
        },
        OutputDest::RawFrame { .. } => FbOutputDest::Raw,
        OutputDest::RawPacket { .. } => FbOutputDest::Encoded,
    };
    let mut fb = FbOutputConfig::new(id.to_string(), av_type, dest);
    if let Some(ref e) = config.encode {
        fb = fb.with_encode(to_fb_encode_config(e));
    }
    Some(fb)
}

fn to_fb_encode_config(e: &EncodeConfig) -> ffmpeg_bus::bus::EncodeConfig {
    ffmpeg_bus::bus::EncodeConfig {
        codec: e.codec.clone(),
        width: e.width,
        height: e.height,
        bitrate: e.bitrate,
        preset: e.preset.clone(),
        pixel_format: e.pixel_format.clone(),
    }
}

/// Forwards ffmpeg-bus VideoFrame stream to lite-nvr RawSinkSource (VideoRawFrame).
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
    }
}

impl PipeConfig {
    pub fn builder() -> PipeConfigBuilder {
        PipeConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct PipeConfigBuilder {
    input: Option<InputConfig>,
    outputs: Vec<OutputConfig>,
}

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
        self.outputs.push(OutputConfig {
            dest: OutputDest::Network {
                url: url.into(),
                format: "rtsp".to_string(),
            },
            encode,
        });
        self
    }

    /// Add direct remux output (no re-encoding)
    pub fn add_remux_output(mut self, url: impl Into<String>, format: impl Into<String>) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::Network {
                url: url.into(),
                format: format.into(),
            },
            encode: None,
        });
        self
    }

    /// Add raw frame output
    pub fn add_raw_frame_output(mut self, sink: Arc<RawSinkSource>) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::RawFrame { sink },
            encode: None,
        });
        self
    }

    /// Add encoded packet output
    pub fn add_raw_packet_output(mut self, sink: Arc<RawSinkSource>, encode: EncodeConfig) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::RawPacket { sink },
            encode: Some(encode),
        });
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
