use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bytes::Bytes;

use crate::media::stream::RawSinkSource;

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
    RawFrame { sink: Arc<RawSinkSource> },
    /// Encoded packet sink，only for encoded packet
    RawPacket { sink: Arc<RawSinkSource> },
    /// ZLMediaKit Media: push raw (demuxed) packets to ZLM
    #[cfg(feature = "zlm")]
    Zlm(Arc<rszlm::media::Media>),
}

/// Configuration for a single output
#[derive(Clone)]
pub struct OutputConfig {
    pub dest: OutputDest,
    /// None = direct remux (no re-encoding), Some = use specified encoding
    pub encode: Option<EncodeConfig>,
}

/// Input configuration
#[derive(Clone)]
pub enum InputConfig {
    Network { url: String },
    File { path: String },
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
