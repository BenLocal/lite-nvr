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

    pub fn new_encoded(data: Vec<u8>, width: u32, height: u32, codec_id: i32) -> Self {
        Self {
            data: Bytes::from(data),
            width: width,
            height: height,
            codec_id: codec_id,
            ..Default::default()
        }
    }

    /// Convert YUV frame to RGB24 data
    /// Auto-detects format based on data size (YUV420P, YUV422P, YUV444P)
    pub fn to_rgb(&self) -> Vec<u8> {
        let width = self.width as usize;
        let height = self.height as usize;
        let y_size = width * height;
        let data_len = self.data.len();

        // Auto-detect format based on data size
        // YUV420P: width * height * 1.5
        // YUV422P: width * height * 2
        // YUV444P: width * height * 3
        let expected_420 = y_size + y_size / 2;
        let expected_422 = y_size * 2;
        let expected_444 = y_size * 3;

        let (uv_width, uv_height, detected_format) = if data_len >= expected_444 {
            (width, height, 5) // YUV444P
        } else if data_len >= expected_422 {
            (width / 2, height, 4) // YUV422P
        } else if data_len >= expected_420 {
            (width / 2, height / 2, 0) // YUV420P
        } else {
            // Fallback to YUV420P
            (width / 2, height / 2, 0)
        };

        let uv_size = uv_width * uv_height;

        let y_plane = &self.data[0..y_size];
        let u_plane = &self.data[y_size..y_size + uv_size];
        let v_plane = &self.data[y_size + uv_size..y_size + uv_size * 2];

        let mut rgb = Vec::with_capacity(width * height * 3);

        for j in 0..height {
            for i in 0..width {
                let y_idx = j * width + i;

                // Calculate UV index based on detected format
                let uv_idx = match detected_format {
                    0 => (j / 2) * uv_width + (i / 2), // YUV420P
                    4 => j * uv_width + (i / 2),       // YUV422P
                    5 => j * uv_width + i,             // YUV444P
                    _ => (j / 2) * uv_width + (i / 2), // Default
                };

                let y = y_plane[y_idx] as f32;
                let u = u_plane[uv_idx] as f32 - 128.0;
                let v = v_plane[uv_idx] as f32 - 128.0;

                // YUV to RGB conversion (BT.601)
                let r = (y + 1.402 * v).clamp(0.0, 255.0) as u8;
                let g = (y - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
                let b = (y + 1.772 * u).clamp(0.0, 255.0) as u8;

                rgb.push(r);
                rgb.push(g);
                rgb.push(b);
            }
        }

        rgb
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
