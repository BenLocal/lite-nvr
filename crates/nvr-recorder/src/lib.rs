//! Record one RTSP source into time-sliced stream-copy segments.

pub mod config;

pub use config::{Container, ReconnectPolicy, RecorderConfig, RtspTransport, TrackSelect};
