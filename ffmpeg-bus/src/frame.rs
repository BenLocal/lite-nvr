use std::fmt::{Display, Formatter};
use std::sync::Arc;

use bytes::Bytes;

use crate::output::OutputMessage;
use crate::packet::RawPacket;

pub type RawFrameSender = tokio::sync::broadcast::Sender<RawFrameCmd>;
pub type RawFrameReceiver = tokio::sync::broadcast::Receiver<RawFrameCmd>;

#[derive(Clone)]
pub enum RawFrameCmd {
    Data(RawFrame),
    EOF,
}

#[derive(Clone)]
pub enum RawFrame {
    Video(RawVideoFrame),
    Audio(RawAudioFrame),
}

#[derive(Clone)]
pub struct RawAudioFrame {
    frame: Arc<ffmpeg_next::frame::Audio>,
}

impl RawAudioFrame {
    pub fn pts(&self) -> Option<i64> {
        self.frame.pts()
    }

    pub fn timestamp(&self) -> Option<i64> {
        self.frame.timestamp()
    }

    pub fn format(&self) -> ffmpeg_next::format::Sample {
        self.frame.format()
    }

    pub fn get_mut(&mut self) -> &mut ffmpeg_next::frame::Audio {
        Arc::make_mut(&mut self.frame)
    }
}

impl From<ffmpeg_next::frame::Audio> for RawAudioFrame {
    fn from(frame: ffmpeg_next::frame::Audio) -> Self {
        Self {
            frame: Arc::new(frame),
        }
    }
}

#[derive(Clone)]
pub struct RawVideoFrame {
    frame: Arc<ffmpeg_next::frame::Video>,
}

impl From<ffmpeg_next::frame::Video> for RawVideoFrame {
    fn from(frame: ffmpeg_next::frame::Video) -> Self {
        Self {
            frame: Arc::new(frame),
        }
    }
}

impl RawVideoFrame {
    pub fn width(&self) -> u32 {
        self.frame.width()
    }

    pub fn height(&self) -> u32 {
        self.frame.height()
    }

    pub fn format(&self) -> ffmpeg_next::format::Pixel {
        self.frame.format()
    }

    pub fn pts(&self) -> Option<i64> {
        self.frame.pts()
    }

    pub fn get_mut(&mut self) -> &mut ffmpeg_next::frame::Video {
        Arc::make_mut(&mut self.frame)
    }

    pub fn data(&self) -> Bytes {
        Bytes::copy_from_slice(self.frame.data(0))
    }

    pub fn is_key(&self) -> bool {
        self.frame.is_key()
    }
}

#[derive(Debug, Default)]
pub struct VideoFrame {
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

impl VideoFrame {
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
}

impl Display for VideoFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "VideoFrame data_len: {}, width: {}, height: {}, format: {}, pts: {}, dts: {}, is_key: {}, codec_id: {}",
            self.data.len(),
            self.width,
            self.height,
            self.format,
            self.pts,
            self.dts,
            self.is_key,
            self.codec_id
        )
    }
}

impl Clone for VideoFrame {
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

impl TryFrom<RawFrame> for VideoFrame {
    type Error = anyhow::Error;
    fn try_from(value: RawFrame) -> Result<Self, Self::Error> {
        if let RawFrame::Video(frame) = value {
            Ok(Self {
                data: frame.data(),
                width: frame.width(),
                height: frame.height(),
                format: frame.format() as i32,
                pts: frame.pts().unwrap_or(0),
                dts: 0,
                is_key: frame.is_key(),
                codec_id: ffmpeg_next::codec::Id::None as i32,
            })
        } else {
            Err(anyhow::anyhow!("not a video frame"))
        }
    }
}

impl From<OutputMessage> for VideoFrame {
    fn from(value: OutputMessage) -> Self {
        Self {
            data: value.data,
            width: value.width,
            height: value.height,
            format: 0,
            pts: value.pts.unwrap_or(0),
            dts: value.dts.unwrap_or(0),
            is_key: value.is_key,
            codec_id: value.codec_id,
        }
    }
}

impl From<RawPacket> for VideoFrame {
    fn from(packet: RawPacket) -> Self {
        Self {
            data: packet.data(),
            width: 0,
            height: 0,
            format: 0,
            pts: packet.pts().unwrap_or(0),
            dts: packet.dts().unwrap_or(0),
            is_key: packet.is_key(),
            codec_id: 0,
        }
    }
}
