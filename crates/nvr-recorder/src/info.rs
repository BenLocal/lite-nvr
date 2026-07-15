use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VideoMeta {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioMeta {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentInfo {
    pub path: PathBuf,
    pub start_wall: DateTime<Utc>,
    pub end_wall: DateTime<Utc>,
    pub duration: f64,
    pub size_bytes: u64,
    pub video: Option<VideoMeta>,
    pub audio: Option<AudioMeta>,
}

/// FFmpeg's canonical short name for a codec id (e.g. "h264", "aac").
pub(crate) fn codec_name(id: ffmpeg_next::codec::Id) -> String {
    unsafe {
        let ptr = ffmpeg_next::ffi::avcodec_get_name(id.into());
        if ptr.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
#[path = "info_test.rs"]
mod info_test;
