//! Synchronous HDMI video/audio output using Linux DRM/KMS and ALSA.
//!
//! Frames use XRGB8888: on little-endian Rockchip, each pixel is B, G, R, X.
//! Call blocking output methods on a dedicated thread, outside async executors.

#[cfg(not(target_os = "linux"))]
use std::path::Path;
use std::path::PathBuf;

mod audio;
pub use audio::{AudioConfig, AudioOutput};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{HdmiOutput, list_connectors};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HDMI output requires Linux DRM/KMS")]
    Unsupported,
    #[error("DRM operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("no connected HDMI connector matching {0:?}")]
    NoConnector(Option<String>),
    #[error("requested display mode is unavailable")]
    NoMode,
    #[error("no available CRTC for this HDMI connector")]
    NoCrtc,
    #[error("invalid frame dimensions, stride, or buffer length")]
    InvalidFrame,
    #[error("page flip timed out; close and reopen the output")]
    FlipTimeout,
    #[error("output is closed or a previous page flip failed")]
    Closed,
    #[error("HDMI audio output requires Linux ALSA")]
    UnsupportedAudio,
    #[error("invalid audio device, sample rate, channels, or interleaved sample count")]
    InvalidAudio,
    #[error("audio playback made no progress for one second")]
    AudioTimeout,
    #[cfg(target_os = "linux")]
    #[error("ALSA operation failed: {0}")]
    Audio(#[from] alsa::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Connector {
    pub id: u32,
    /// DRM connector name, e.g. HDMI-A-1 (not necessarily the board's socket label).
    pub name: String,
    pub state: ConnectionState,
    pub modes: Vec<DisplayMode>,
}

#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub card: PathBuf,
    /// None selects the first connected HDMI connector.
    pub connector: Option<String>,
    /// None selects the preferred mode advertised by the HDMI sink.
    pub mode: Option<DisplayMode>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            card: PathBuf::from("/dev/dri/card0"),
            connector: None,
            mode: None,
        }
    }
}

/// Borrowed XRGB8888 frame. Dimensions must match the selected display mode.
#[derive(Debug, Clone, Copy)]
pub struct VideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: &'a [u8],
}

#[cfg(any(target_os = "linux", test))]
fn layout(width: u32, height: u32, stride: usize) -> Result<(usize, usize)> {
    let row = usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(4))
        .filter(|row| *row > 0 && *row <= stride)
        .ok_or(Error::InvalidFrame)?;
    let rows = usize::try_from(height)
        .ok()
        .filter(|h| *h > 0)
        .ok_or(Error::InvalidFrame)?;
    let length = (rows - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(row))
        .ok_or(Error::InvalidFrame)?;
    Ok((row, length))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn copy_frame(
    frame: VideoFrame<'_>,
    mode: DisplayMode,
    target: &mut [u8],
    pitch: usize,
) -> Result<()> {
    if (frame.width, frame.height) != (mode.width, mode.height) {
        return Err(Error::InvalidFrame);
    }
    let (row, source_length) = layout(frame.width, frame.height, frame.stride)?;
    let (_, target_length) = layout(mode.width, mode.height, pitch)?;
    if frame.data.len() < source_length || target.len() < target_length {
        return Err(Error::InvalidFrame);
    }
    for (source, destination) in frame
        .data
        .chunks(frame.stride)
        .zip(target.chunks_mut(pitch))
        .take(usize::try_from(mode.height).map_err(|_| Error::InvalidFrame)?)
    {
        destination[..row].copy_from_slice(&source[..row]);
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn list_connectors(_card: impl AsRef<Path>) -> Result<Vec<Connector>> {
    Err(Error::Unsupported)
}

#[cfg(not(target_os = "linux"))]
pub struct HdmiOutput {
    _private: (),
}

#[cfg(not(target_os = "linux"))]
impl HdmiOutput {
    pub fn open(_config: &OutputConfig) -> Result<Self> {
        Err(Error::Unsupported)
    }
    pub fn open_sibling(&self, _connector: &str, _mode: Option<DisplayMode>) -> Result<Self> {
        Err(Error::Unsupported)
    }
    pub fn mode(&self) -> DisplayMode {
        unreachable!("HDMI output cannot be constructed outside Linux")
    }
    pub fn present(&mut self, _frame: VideoFrame<'_>) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub fn close(&mut self) -> Result<()> {
        Err(Error::Unsupported)
    }
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod lib_test;
