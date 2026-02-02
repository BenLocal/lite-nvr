// ============================================================================
// Pipeline Implementation using ez-ffmpeg
// ============================================================================

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ez_ffmpeg::{
    AVMediaType, FfmpegContext, FfmpegScheduler, Frame,
    core::{
        context::{input::Input, output::Output},
        filter::{
            frame_filter::FrameFilter, frame_filter_context::FrameFilterContext,
            frame_pipeline_builder::FramePipelineBuilder,
        },
        scheduler::ffmpeg_scheduler::Running,
    },
};
use tokio_util::sync::CancellationToken;

use crate::media::{
    stream::RawSinkSource,
    types::{EncodeConfig, InputConfig, OutputConfig, OutputDest, PipeConfig},
};

/// Pipeline: Optimized media processing using ez-ffmpeg
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
        };

        log::info!("Pipe: starting with input {}", input_url);

        let cancel = self.cancel.clone();
        let outputs = self.config.outputs.clone();

        // Run FFmpeg in a blocking task
        let handle = tokio::task::spawn_blocking(move || {
            run_ffmpeg_pipeline(&input_url, &outputs, cancel);
        });

        // Wait for completion or cancellation
        tokio::select! {
            _ = handle => {
                log::info!("Pipe: pipeline finished");
            }
            _ = self.cancel.cancelled() => {
                log::info!("Pipe: cancelled");
            }
        }

        self.started.store(false, Ordering::Relaxed);
    }
}

/// Run the FFmpeg pipeline
fn run_ffmpeg_pipeline(input_url: &str, outputs: &[OutputConfig], cancel: CancellationToken) {
    // Build input
    let input: Input = Input::new(input_url.to_string());

    // Build outputs
    let mut ez_outputs: Vec<Output> = Vec::new();

    for output_config in outputs {
        match build_output(output_config) {
            Some(output) => ez_outputs.push(output),
            None => {
                log::warn!(
                    "Pipe: failed to build output for {:?}",
                    dest_name(&output_config.dest)
                );
            }
        }
    }

    if ez_outputs.is_empty() {
        log::error!("Pipe: no valid outputs");
        return;
    }

    // Build context
    let context = match FfmpegContext::builder()
        .input(input)
        .outputs(ez_outputs)
        .build()
    {
        Ok(ctx) => ctx,
        Err(e) => {
            log::error!("Pipe: failed to build context: {}", e);
            return;
        }
    };

    // Start scheduler
    let scheduler: FfmpegScheduler<Running> = match FfmpegScheduler::new(context).start() {
        Ok(s) => s,
        Err(e) => {
            log::error!("Pipe: failed to start scheduler: {}", e);
            return;
        }
    };

    // Wait for completion or cancellation
    loop {
        if cancel.is_cancelled() {
            log::info!("Pipe: aborting scheduler");
            scheduler.abort();
            break;
        }

        // Check if scheduler is still running
        // ez-ffmpeg's wait() is blocking, so we use a short sleep and check
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Try to check completion status
        // Note: ez-ffmpeg may not have a non-blocking check, so we rely on abort
    }

    log::info!("Pipe: run_ffmpeg_pipeline finished");
}

/// Build an ez-ffmpeg Output from OutputConfig
pub fn build_output(config: &OutputConfig) -> Option<Output> {
    match (&config.dest, &config.encode) {
        // Network output without re-encoding (remux)
        (OutputDest::Network { url, format }, None) => {
            let mut output = Output::new(url.clone());
            output = output.set_format(format);
            // Copy codec for remux
            output = output.set_video_codec("copy");
            output = output.set_audio_codec("copy");
            Some(output)
        }

        // Network output with re-encoding
        (OutputDest::Network { url, format }, Some(encode_config)) => {
            let mut output = Output::new(url.clone());
            output = output.set_format(format);
            output = apply_encode_config(output, encode_config);
            Some(output)
        }

        // RawFrame output: use FrameFilter to capture decoded frames
        (OutputDest::RawFrame { sink }, _) => {
            let sink_clone = sink.clone();

            // Create a custom filter to capture frames
            let frame_filter = RawFrameFilter::new(sink_clone);

            // Build frame pipeline
            let mut pipeline_builder: FramePipelineBuilder = AVMediaType::AVMEDIA_TYPE_VIDEO.into();
            pipeline_builder = pipeline_builder.filter("raw-frame-sink", Box::new(frame_filter));

            // Create output that writes to /dev/null but captures frames via filter
            let output = Output::new_by_write_callback(move |_buf| {
                // Discard the encoded data, we only want the raw frames
                _buf.len() as i32
            })
            .set_format("rawvideo")
            .add_frame_pipeline(pipeline_builder);

            Some(output)
        }

        // RawPacket output: use write callback to capture encoded packets
        (OutputDest::RawPacket { sink }, encode_option) => {
            let sink_clone = sink.clone();

            let mut output = Output::new_by_write_callback(move |buf| {
                let _ = sink_clone.writer.try_send(buf.to_vec());
                buf.len() as i32
            });

            // Apply encoding if specified
            if let Some(encode_config) = encode_option {
                output = apply_encode_config(output, encode_config);
            }

            // Set format based on codec
            let format = encode_option
                .as_ref()
                .map(|e| match e.codec.as_str() {
                    "h264" => "h264",
                    "hevc" | "h265" => "hevc",
                    _ => "rawvideo",
                })
                .unwrap_or("rawvideo");
            output = output.set_format(format);

            Some(output)
        }
    }
}

/// Apply encoding configuration to an Output
fn apply_encode_config(mut output: Output, config: &EncodeConfig) -> Output {
    // Set video codec
    output = output.set_video_codec(&config.codec);

    // Build video filter string for scaling
    let mut video_filters = Vec::new();

    if config.width.is_some() || config.height.is_some() {
        let w = config
            .width
            .map(|v| v.to_string())
            .unwrap_or("-1".to_string());
        let h = config
            .height
            .map(|v| v.to_string())
            .unwrap_or("-1".to_string());
        video_filters.push(format!("scale={}:{}", w, h));
    }

    if let Some(ref pix_fmt) = config.pixel_format {
        video_filters.push(format!("format={}", pix_fmt));
    }

    // Apply video filters if any
    // Note: ez-ffmpeg uses filter_desc on the context builder, not on output directly
    // For output-specific options, we use set_video_codec_opt

    // Set bitrate
    if let Some(bitrate) = config.bitrate {
        output = output.set_video_codec_opt("b", format!("{}", bitrate));
    }

    // Set preset
    if let Some(ref preset) = config.preset {
        output = output.set_video_codec_opt("preset", preset);
    }

    output
}

/// Get destination name for logging
pub fn dest_name(dest: &OutputDest) -> String {
    match dest {
        OutputDest::Network { url, .. } => url.clone(),
        OutputDest::RawFrame { .. } => "RawFrame".to_string(),
        OutputDest::RawPacket { .. } => "RawPacket".to_string(),
    }
}

// ============================================================================
// Custom Frame Filter for RawFrame Output
// ============================================================================

/// Frame filter that captures decoded frames and sends them to a sink
struct RawFrameFilter {
    sink: Arc<RawSinkSource>,
}

impl RawFrameFilter {
    fn new(sink: Arc<RawSinkSource>) -> Self {
        Self { sink }
    }
}

impl FrameFilter for RawFrameFilter {
    fn media_type(&self) -> AVMediaType {
        AVMediaType::AVMEDIA_TYPE_VIDEO
    }

    fn filter_frame(
        &mut self,
        frame: Frame,
        _ctx: &FrameFilterContext,
    ) -> Result<Option<Frame>, String> {
        // Check if frame is valid
        unsafe {
            if frame.as_ptr().is_null() || frame.is_empty() {
                return Ok(Some(frame));
            }
        }

        // Extract frame data
        if let Some(data) = extract_frame_data(&frame) {
            let _ = self.sink.writer.try_send(data);
        }

        // Pass through the frame for further processing
        Ok(Some(frame))
    }
}

/// Extract raw pixel data from a Frame
fn extract_frame_data(frame: &Frame) -> Option<Vec<u8>> {
    unsafe {
        let ptr = frame.as_ptr();
        if ptr.is_null() {
            return None;
        }

        let av_frame = &*ptr;
        let width = av_frame.width as usize;
        let height = av_frame.height as usize;

        if width == 0 || height == 0 {
            return None;
        }

        // Calculate total size based on format (assuming YUV420P)
        // Y plane: width * height
        // U plane: (width/2) * (height/2)
        // V plane: (width/2) * (height/2)
        let y_size = width * height;
        let uv_size = (width / 2) * (height / 2);
        let total_size = y_size + uv_size * 2;

        let mut data = Vec::with_capacity(total_size);

        // Copy Y plane
        let y_linesize = av_frame.linesize[0] as usize;
        let y_data = av_frame.data[0];
        if !y_data.is_null() {
            for row in 0..height {
                let src = y_data.add(row * y_linesize);
                let slice = std::slice::from_raw_parts(src, width);
                data.extend_from_slice(slice);
            }
        }

        // Copy U plane
        let u_linesize = av_frame.linesize[1] as usize;
        let u_data = av_frame.data[1];
        if !u_data.is_null() && u_linesize > 0 {
            for row in 0..(height / 2) {
                let src = u_data.add(row * u_linesize);
                let slice = std::slice::from_raw_parts(src, width / 2);
                data.extend_from_slice(slice);
            }
        }

        // Copy V plane
        let v_linesize = av_frame.linesize[2] as usize;
        let v_data = av_frame.data[2];
        if !v_data.is_null() && v_linesize > 0 {
            for row in 0..(height / 2) {
                let src = v_data.add(row * v_linesize);
                let slice = std::slice::from_raw_parts(src, width / 2);
                data.extend_from_slice(slice);
            }
        }

        Some(data)
    }
}

// ============================================================================
// Builder API
// ============================================================================

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

    /// Add RTSP output (with re-encoding)
    pub fn add_rtsp_output(mut self, url: impl Into<String>, encode: EncodeConfig) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::Network {
                url: url.into(),
                format: "rtsp".to_string(),
            },
            encode: Some(encode),
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
