use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bytes::Bytes;
use ffmpeg_bus::bus::{OutputAvType, VideoRawFrameStream};
use ffmpeg_bus::stream::AvStream;
use tokio::task::JoinHandle;

use crate::stream::RawSinkSource;

use ffmpeg_bus::bus::{OutputConfig as FbOutputConfig, OutputDest as FbOutputDest};

/// A sink for a `Demuxed` (raw passthrough) output.
///
/// The [`Pipe`](crate::pipe::Pipe) hands over an accepted output's codec
/// metadata (`av`) and its demuxed packet stream; the sink spawns its own
/// forwarding task and returns the `JoinHandle`. This is the boundary that lets
/// the core stay free of any specific media server (e.g. ZLMediaKit) — the
/// implementation lives in a separate crate.
pub trait DemuxedSink: Send + Sync + 'static {
    /// Begin forwarding the demuxed `stream` (with codec metadata `av`).
    fn start(&self, av: AvStream, stream: VideoRawFrameStream) -> JoinHandle<()>;

    /// Called when the Pipe could not create this output (e.g. the input has no
    /// matching stream). Lets a sink that coordinates with siblings (such as a
    /// video+audio pair sharing one media) drop the missing one from its
    /// expected set. Default: no-op.
    fn on_rejected(&self) {}
}

/// Encode configuration (used as HashMap key, same config shares encoder)
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

impl Hash for EncodeConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.codec.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.bitrate.hash(state);
        self.preset.hash(state);
        self.pixel_format.hash(state);
    }
}

/// Output destination
#[derive(Clone)]
pub enum OutputDest {
    /// Network streaming (RTSP/RTMP/HLS...)
    Network { url: String, format: String },
    /// Raw frame data sink，only for decoded frame
    #[allow(dead_code)]
    RawFrame { sink: Arc<RawSinkSource> },
    /// Encoded packet sink，only for encoded packet
    #[allow(dead_code)]
    RawPacket { sink: Arc<RawSinkSource> },
    /// Demuxed (raw codec) passthrough delivered to a [`DemuxedSink`], e.g. a
    /// ZLMediaKit media. One demuxed input packet per emitted item.
    Demuxed { sink: Arc<dyn DemuxedSink> },
}

/// Configuration for a single output
#[derive(Clone)]
pub struct OutputConfig {
    /// Unique identifier for the output
    /// if None, the output will be identified by the index of the output in the pipeline
    pub id: Option<String>,
    /// Destination of the output
    pub dest: OutputDest,
    /// None = direct remux (no re-encoding), Some = use specified encoding
    pub encode: Option<EncodeConfig>,
    /// Stream type: Video or Audio
    pub av_type: OutputAvType,
    /// Include audio stream in File/Net mux outputs
    pub include_audio: bool,
}

impl OutputConfig {
    pub fn new(dest: OutputDest, encode: Option<EncodeConfig>) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        Self {
            id: Some(id),
            dest,
            encode,
            av_type: OutputAvType::Video,
            include_audio: false,
        }
    }

    #[allow(dead_code)]
    pub fn new_with_id(id: &str, dest: OutputDest, encode: Option<EncodeConfig>) -> Self {
        Self {
            id: Some(id.to_string()),
            dest,
            encode,
            av_type: OutputAvType::Video,
            include_audio: false,
        }
    }

    pub fn with_av_type(mut self, av_type: OutputAvType) -> Self {
        self.av_type = av_type;
        self
    }

    #[allow(dead_code)]
    pub fn with_audio(mut self) -> Self {
        self.include_audio = true;
        self
    }
}

/// Input configuration
#[derive(Clone)]
pub enum InputConfig {
    Network { url: String },
    File { path: String },
    Device { display: String, format: String },
}

impl Into<ffmpeg_bus::bus::InputConfig> for InputConfig {
    fn into(self) -> ffmpeg_bus::bus::InputConfig {
        match self {
            InputConfig::Network { url } => ffmpeg_bus::bus::InputConfig::Net { url },
            InputConfig::File { path } => ffmpeg_bus::bus::InputConfig::File { path },
            InputConfig::Device { display, format } => {
                ffmpeg_bus::bus::InputConfig::Device { display, format }
            }
        }
    }
}

/// Pipeline configuration
pub struct PipeConfig {
    pub input: InputConfig,
    pub outputs: Vec<OutputConfig>,
}

#[derive(Debug, Default)]
pub struct VideoRawFrame {
    pub data: Bytes,
    pub width: u32,
    pub height: u32,
    // AVPixelFormat
    pub format: i32,
    pub pts: i64,
    pub dts: i64,
    pub is_key: bool,
    // AVCodecID
    pub codec_id: i32,
}

impl VideoRawFrame {
    pub fn new(
        data: Vec<u8>,
        width: u32,
        height: u32,
        format: i32,
        pts: i64,
        dts: i64,
        is_key: bool,
        codec_id: i32,
    ) -> Self {
        Self {
            data: Bytes::from(data),
            width,
            height,
            format,
            pts,
            dts,
            is_key,
            codec_id,
        }
    }
}

impl Display for VideoRawFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "VideoRawFrame {{ data: {} }}", self.data.len())
    }
}

impl Clone for VideoRawFrame {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            width: self.width,
            height: self.height,
            format: self.format,
            pts: self.pts,
            dts: self.dts,
            is_key: self.is_key,
            codec_id: self.codec_id,
        }
    }
}

impl Into<Option<FbOutputConfig>> for OutputConfig {
    fn into(self) -> Option<FbOutputConfig> {
        to_fb_output(&self)
    }
}

fn to_fb_output(config: &OutputConfig) -> Option<FbOutputConfig> {
    let av_type = config.av_type;
    let dest = match &config.dest {
        OutputDest::Network { url, format } => FbOutputDest::Net {
            url: url.clone(),
            format: Some(format.clone()),
        },
        OutputDest::RawFrame { .. } => FbOutputDest::Raw,
        OutputDest::RawPacket { .. } => FbOutputDest::Encoded,
        // A demuxed sink (e.g. ZLM) consumes raw codec frames directly (no
        // container framing). `Demuxed` gives one demuxed input packet per
        // emitted item with no re-encoding or muxing, so video gets clean
        // Annex B / AVCC NALs and audio gets one raw AAC frame per packet.
        OutputDest::Demuxed { .. } => FbOutputDest::Demuxed,
    };
    let id = config
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut fb = FbOutputConfig::new(id.to_string(), av_type, dest);
    if let Some(ref e) = config.encode {
        fb = fb.with_encode(to_fb_encode_config(e));
    }
    if config.include_audio {
        fb = fb.with_audio();
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
        sample_rate: None,
        channels: None,
        audio_bitrate: None,
    }
}
