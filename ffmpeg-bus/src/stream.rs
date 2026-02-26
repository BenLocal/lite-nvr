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

    pub fn width(&self) -> u32 {
        unsafe {
            let ptr = self.parameters.as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
            (*ptr).width.max(0) as u32
        }
    }

    pub fn height(&self) -> u32 {
        unsafe {
            let ptr = self.parameters.as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
            (*ptr).height.max(0) as u32
        }
    }

    pub fn fps(&self) -> f32 {
        self.rate.numerator() as f32 / self.rate.denominator() as f32
    }

    /// Build an AvStream suitable for mux encoder output: same dimensions/time_base/rate as
    /// `input`, but with `codec_id` (e.g. H264). Used when muxing encoded packets.
    pub fn for_encoder_output(input: &AvStream, codec_id: ffmpeg_next::codec::Id) -> Self {
        let params = input.parameters().clone();
        unsafe {
            let ptr = params.as_ptr() as *mut ffmpeg_next::ffi::AVCodecParameters;
            (*ptr).codec_type = ffmpeg_next::media::Type::Video.into();
            (*ptr).codec_id = codec_id.into();
        }
        Self {
            index: 0,
            parameters: params,
            time_base: input.time_base(),
            rate: input.rate(),
        }
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
