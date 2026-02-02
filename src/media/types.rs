use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bytes::Bytes;

use crate::media::stream::RawSinkSource;

/// Raw encoded packet (after demux, before decode)
#[derive(Clone, Debug)]
pub struct RawPacket {
    pub stream_index: usize,
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub is_key: bool,
    pub codec_id: i32, // AVCodecID
}

/// Decoded video frame
#[derive(Clone, Debug)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub format: i32, // AVPixelFormat
    pub data: Vec<u8>,
    pub linesize: [i32; 4],
    pub pts: i64,
}

/// Encoded packet (after encode)
#[derive(Clone, Debug)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub is_key: bool,
}

// ============================================================================
// Configuration Types
// ============================================================================

/// Encode configuration (used as HashMap key, same config shares encoder)
#[derive(Clone, Debug)]
pub struct EncodeConfig {
    pub codec: String,                // "h264", "hevc", "rawvideo"
    pub width: Option<u32>,           // None = keep original
    pub height: Option<u32>,          // None = keep original
    pub bitrate: Option<u64>,         // bps
    pub preset: Option<String>,       // "ultrafast", "medium", etc.
    pub pixel_format: Option<String>, // "yuv420p", "rgb24", etc.
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
    /// Raw frame data sink
    RawFrame { sink: Arc<RawSinkSource> },
    /// Encoded packet sink
    RawPacket { sink: Arc<RawSinkSource> },
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
}

/// Pipeline configuration
pub struct PipeConfig {
    pub input: InputConfig,
    pub outputs: Vec<OutputConfig>,
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
