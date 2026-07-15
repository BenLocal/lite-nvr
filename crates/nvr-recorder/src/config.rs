use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtspTransport {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSelect {
    Video,
    Audio,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Ts,
    Mp4,
    Mkv,
}

impl Container {
    /// File extension for this container.
    pub fn extension(self) -> &'static str {
        match self {
            Container::Ts => "ts",
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
        }
    }

    /// FFmpeg muxer (format) name for this container.
    pub fn muxer_name(self) -> &'static str {
        match self {
            Container::Ts => "mpegts",
            Container::Mp4 => "mp4",
            Container::Mkv => "matroska",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// `None` = reconnect forever (until cancelled); `Some(0)` = never reconnect.
    pub max_retries: Option<u32>,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_retries: None,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(16),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub url: String,
    pub transport: RtspTransport,
    pub tracks: TrackSelect,
    pub segment_time: Duration,
    pub align_to_wall_clock: bool,
    pub container: Container,
    pub output_dir: PathBuf,
    /// strftime pattern for the segment start wall-clock (no extension).
    pub filename_pattern: String,
    pub open_timeout: Duration,
    pub reconnect: ReconnectPolicy,
}

impl RecorderConfig {
    /// Build a config with documented defaults; override fields as needed.
    pub fn new(url: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            transport: RtspTransport::Tcp,
            tracks: TrackSelect::Both,
            segment_time: Duration::from_secs(60),
            align_to_wall_clock: false,
            container: Container::Ts,
            output_dir: output_dir.into(),
            filename_pattern: "rec_%Y%m%d_%H%M%S".to_string(),
            open_timeout: Duration::from_secs(5),
            reconnect: ReconnectPolicy::default(),
        }
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
