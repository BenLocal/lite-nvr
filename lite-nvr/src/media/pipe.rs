use std::{
    backtrace::Backtrace,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ffmpeg_bus::bus::{Bus as FbBus, VideoRawFrameStream};
use futures::StreamExt;
#[cfg(feature = "zlm")]
use rszlm::{
    frame::Frame as ZlmFrame,
    obj::{CodecArgs, CodecId, Track, VideoCodecArgs},
};
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

        // Map and add input
        let fb_input = self.config.input.clone().into();
        if let Err(e) = bus.add_input(fb_input, None).await {
            log::error!(
                "Pipe: add_input failed: {:#}\nbacktrace:\n{}",
                e,
                Backtrace::capture()
            );
            self.started.store(false, Ordering::Relaxed);
            return;
        }

        // Add each output and optionally forward stream to sink
        let mut join_handles = Vec::new();
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
                        #[cfg(feature = "zlm")]
                        OutputDest::Zlm(media) => {
                            let media = Arc::clone(media);
                            let handle = tokio::spawn(async move {
                                forward_raw_packet_stream_to_zlm(stream, av, media).await;
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

/// Forward raw (demuxed) packet stream from ffmpeg-bus to ZLMediaKit Media.
/// The ffmpeg-bus Mux output with format "h264" uses a large buffer (256KB) so each
/// chunk is complete NALUs (Annex B). PTS/DTS are converted to milliseconds.
/// Automatically converts H.264/H.265 from MP4 (AVCC/HVCC) to Annex B format if needed.
///
/// Time base: the **input** stream may have any time_base (e.g. testsrc 1/10), but the
/// frames we receive here come from the **encoder** path (Mux from encoder). The encoder
/// uses time_base 1/90000, so frame.pts/dts are in 90kHz units; we convert to ms by /90.
#[cfg(feature = "zlm")]
async fn forward_raw_packet_stream_to_zlm(
    mut stream: VideoRawFrameStream,
    av: ffmpeg_bus::stream::AvStream,
    media: Arc<rszlm::media::Media>,
) {
    use ffmpeg_bus::bsf::{convert_avcc_to_annexb, is_annexb_packet};

    let default_width = av.width();
    let default_height = av.height();
    let default_fps = av.fps();
    let mut track_initialized = false;
    let mut needs_conversion = false;
    let mut conversion_checked = false;

    while let Some(opt) = stream.next().await {
        let Some(frame) = opt else { continue };

        let (w, h) = (
            if frame.width > 0 {
                frame.width as i32
            } else {
                default_width as i32
            },
            if frame.height > 0 {
                frame.height as i32
            } else {
                default_height as i32
            },
        );

        // Wait for second frame to estimate fps, then init track once with correct fps
        if !track_initialized {
            media.init_track(&Track::new(
                CodecId::H264,
                Some(CodecArgs::Video(VideoCodecArgs {
                    width: w,
                    height: h,
                    fps: default_fps,
                })),
            ));
            media.init_complete();
            track_initialized = true;
            log::info!("ZLM: track initialized ({}x{}, fps={})", w, h, default_fps);

            // Conversion check (use current frame; same stream as first)
            if !conversion_checked {
                let packet_data = frame.data.as_ref();
                needs_conversion = !is_annexb_packet(packet_data);
                conversion_checked = true;
                log::info!(
                    "ZLM: {}",
                    if needs_conversion {
                        "detected MP4 format, will use BSF conversion"
                    } else {
                        "detected Annex B format, no conversion needed"
                    }
                );
            }
        }

        // Normalize to 1/90000 then to ms: if time_base != 1/90000, rescale pts/dts first
        let time_base = av.time_base();
        let pts_ms = frame.pts_90k_to_ms(time_base);
        let dts_ms = frame.dts_90k_to_ms(time_base);

        // Get packet data (convert AVCC to Annex B if needed)
        let data: std::borrow::Cow<'_, [u8]> = if needs_conversion {
            std::borrow::Cow::Owned(convert_avcc_to_annexb(frame.data.as_ref()).to_vec())
        } else {
            std::borrow::Cow::Borrowed(frame.data.as_ref())
        };

        let zlm_frame = ZlmFrame::new(CodecId::H264, dts_ms as u64, pts_ms as u64, data.as_ref());
        if !media.input_frame(&zlm_frame) {
            log::warn!(
                "ZLM: input_frame failed: pts_ms={} dts_ms={} len={} is_key={}",
                pts_ms,
                dts_ms,
                data.len(),
                frame.is_key
            );
        }
    }

    log::info!("ZLM: stream ended");
}

/// Get destination name for logging (used by tests).
pub fn dest_name(dest: &OutputDest) -> String {
    match dest {
        OutputDest::Network { url, .. } => url.clone(),
        OutputDest::RawFrame { .. } => "RawFrame".to_string(),
        OutputDest::RawPacket { .. } => "RawPacket".to_string(),
        #[cfg(feature = "zlm")]
        OutputDest::Zlm(_) => "Zlm".to_string(),
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

    /// Add zlm output
    #[cfg(feature = "zlm")]
    pub fn add_zlm_output(mut self, media: Arc<rszlm::media::Media>) -> Self {
        self.outputs
            .push(OutputConfig::new(OutputDest::Zlm(media), None));
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
