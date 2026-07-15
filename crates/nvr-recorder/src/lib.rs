//! Record one RTSP source into time-sliced stream-copy segments.

pub mod config;
pub mod info;

pub use config::{Container, ReconnectPolicy, RecorderConfig, RtspTransport, TrackSelect};
pub use info::{AudioMeta, SegmentInfo, VideoMeta};
