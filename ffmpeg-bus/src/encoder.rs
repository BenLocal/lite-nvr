use ffmpeg_next::{Dictionary, Rational};

use crate::stream::AvStream;

pub enum EncoderType {
    Video(ffmpeg_next::codec::encoder::Video),
    Audio(ffmpeg_next::codec::encoder::Audio),
}

#[derive(Debug, Clone)]
pub struct Settings {
    width: u32,
    height: u32,
    keyframe_interval: u64,
    fps: Rational,
    codec: Option<String>,
}

pub struct Encoder {
    stream: AvStream,
    inner: EncoderType,
    encoder_time_base: Rational,
}

impl Encoder {
    pub fn new(
        stream: &AvStream,
        settings: Settings,
        options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        let mut encoder_context = match settings.codec {
            Some(codec) => {
                let codec = ffmpeg_next::encoder::find_by_name(&codec)
                    .ok_or(anyhow::anyhow!("codec not found"))?;
                ffmpeg_next::codec::Context::new_with_codec(codec)
            }
            None => ffmpeg_next::codec::Context::new(),
        };

        let mut encoder = encoder_context.encoder().video()?;
        // settings.apply_to(&mut encoder);

        // // Just use the ffmpeg global time base which is precise enough
        // // that we should never get in trouble.
        // encoder.set_time_base(TIME_BASE);

        let encoder = encoder.open_with(options.unwrap_or_default())?;
        //let encoder_time_base = ffi::get_encoder_time_base(&encoder);
        Ok(Self {
            stream: stream.clone(),
            inner: EncoderType::Video(encoder),
            encoder_time_base: stream.time_base(),
        })
    }
}
