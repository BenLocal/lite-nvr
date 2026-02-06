use ffmpeg_next::{Rational, codec::Parameters, format::stream};

unsafe impl Send for AvStream {}
unsafe impl Sync for AvStream {}

pub struct AvStream {
    index: usize,
    parameters: Parameters,
    time_base: Rational,
    rate: Rational,
}

impl AvStream {
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }
    pub fn time_base(&self) -> Rational {
        self.time_base
    }
    pub fn rate(&self) -> Rational {
        self.rate
    }

    pub fn is_video(&self) -> bool {
        self.parameters.medium() == ffmpeg_next::media::Type::Video
    }

    pub fn is_audio(&self) -> bool {
        self.parameters.medium() == ffmpeg_next::media::Type::Audio
    }
}

impl From<stream::Stream<'_>> for AvStream {
    fn from(stream: stream::Stream<'_>) -> Self {
        Self {
            index: stream.index(),
            parameters: stream.parameters(),
            time_base: stream.time_base(),
            rate: stream.avg_frame_rate(),
        }
    }
}

impl Clone for AvStream {
    fn clone(&self) -> Self {
        Self {
            index: self.index,
            parameters: self.parameters.clone(),
            time_base: self.time_base,
            rate: self.rate,
        }
    }
}
