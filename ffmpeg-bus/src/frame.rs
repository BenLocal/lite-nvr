use std::sync::Arc;

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
}
