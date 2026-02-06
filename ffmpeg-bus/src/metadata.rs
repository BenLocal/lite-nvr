//! Media file metadata (similar to ffprobe).

use std::fmt;

use crate::stream::AvStream;

/// Format-level info (corresponds to ffprobe format).
#[derive(Debug, Clone)]
pub struct FormatInfo {
    /// Format name, e.g. "mov,mp4,m4a,3gp,3g2,mj2"
    pub format_name: String,
    /// Duration in seconds; None if unknown (e.g. raw h264).
    pub duration_sec: Option<f64>,
    /// Total bitrate in bps; 0 if unknown.
    pub bit_rate: i64,
    /// Number of streams.
    pub nb_streams: u32,
}

/// Per-stream info (corresponds to ffprobe stream).
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// Stream index.
    pub index: usize,
    /// Type: "video" | "audio" | "subtitle" etc.
    pub codec_type: String,
    /// Codec name, e.g. "h264", "aac"
    pub codec_name: String,
    /// Time base, e.g. "1/90000"
    pub time_base: String,
    /// Stream duration in time_base units; None if unknown.
    pub duration_ts: Option<i64>,
    /// Frame rate / sample rate etc., e.g. "10/1"
    pub rate: String,
    /// Video only: width.
    pub width: Option<u32>,
    /// Video only: height.
    pub height: Option<u32>,
    /// Audio only: sample rate.
    pub sample_rate: Option<u32>,
    /// Audio only: channel count.
    pub channels: Option<u32>,
}

/// Full probe result (format + streams, like ffprobe).
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub format: FormatInfo,
    pub streams: Vec<StreamInfo>,
}

impl fmt::Display for MediaInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "[FORMAT]")?;
        writeln!(f, "format_name={}", self.format.format_name)?;
        if let Some(d) = self.format.duration_sec {
            writeln!(f, "duration_sec={:.3}", d)?;
        } else {
            writeln!(f, "duration_sec=N/A")?;
        }
        writeln!(f, "bit_rate={}", self.format.bit_rate)?;
        writeln!(f, "nb_streams={}", self.format.nb_streams)?;
        writeln!(f, "[/FORMAT]")?;
        for s in &self.streams {
            writeln!(f, "[STREAM]")?;
            writeln!(f, "index={}", s.index)?;
            writeln!(f, "codec_type={}", s.codec_type)?;
            writeln!(f, "codec_name={}", s.codec_name)?;
            writeln!(f, "time_base={}", s.time_base)?;
            if let Some(d) = s.duration_ts {
                writeln!(f, "duration_ts={}", d)?;
            }
            writeln!(f, "rate={}", s.rate)?;
            if let Some(w) = s.width {
                writeln!(f, "width={}", w)?;
            }
            if let Some(h) = s.height {
                writeln!(f, "height={}", h)?;
            }
            if let Some(sr) = s.sample_rate {
                writeln!(f, "sample_rate={}", sr)?;
            }
            if let Some(c) = s.channels {
                writeln!(f, "channels={}", c)?;
            }
            writeln!(f, "[/STREAM]")?;
        }
        Ok(())
    }
}

/// Opens a file and returns media metadata (similar to ffprobe).
///
/// # Example
///
/// ```ignore
/// use ffmpeg_bus::probe;
/// let info = probe("input.mp4")?;
/// println!("{}", info);
/// ```
pub fn probe(path: &str) -> anyhow::Result<MediaInfo> {
    let input = ffmpeg_next::format::input(path)?;

    let format_name = input.format().name().to_string();
    let nb_streams = input.nb_streams();
    let bit_rate = input.bit_rate();
    // AV_TIME_BASE = 1_000_000; duration is in 1/AV_TIME_BASE seconds
    let duration_sec = {
        let d = input.duration();
        if d == ffmpeg_next::ffi::AV_NOPTS_VALUE as i64 || d <= 0 {
            None
        } else {
            Some(d as f64 / 1_000_000.0)
        }
    };

    let mut streams = Vec::with_capacity(nb_streams as usize);
    for i in 0..nb_streams as usize {
        let stream = input.stream(i).ok_or_else(|| anyhow::anyhow!("stream {} not found", i))?;
        let duration_ts = {
            let d = stream.duration();
            if d == ffmpeg_next::ffi::AV_NOPTS_VALUE as i64 || d < 0 {
                None
            } else {
                Some(d)
            }
        };
        let av_stream = AvStream::from(stream);
        let params = av_stream.parameters();
        let medium = params.medium();
        let codec_type = format!("{:?}", medium).to_lowercase();
        let codec_name = format!("{:?}", params.id()).to_lowercase();
        let time_base = av_stream.time_base();
        let time_base_str = format!("{}/{}", time_base.numerator(), time_base.denominator());
        let rate = av_stream.rate();
        let rate_str = format!("{}/{}", rate.numerator(), rate.denominator());

        let (width, height, sample_rate, channels) = if av_stream.is_video() {
            let (w, h) = video_size_from_parameters(params);
            (Some(w), Some(h), None, None)
        } else if av_stream.is_audio() {
            let (sr, ch) = audio_params_from_parameters(params);
            (None, None, Some(sr), Some(ch))
        } else {
            (None, None, None, None)
        };

        streams.push(StreamInfo {
            index: av_stream.index(),
            codec_type,
            codec_name,
            time_base: time_base_str,
            duration_ts,
            rate: rate_str,
            width,
            height,
            sample_rate,
            channels,
        });
    }

    Ok(MediaInfo {
        format: FormatInfo {
            format_name,
            duration_sec,
            bit_rate,
            nb_streams,
        },
        streams,
    })
}

/// Reads video width/height from codec parameters (not exposed by ffmpeg-next).
fn video_size_from_parameters(params: &ffmpeg_next::codec::Parameters) -> (u32, u32) {
    unsafe {
        let ptr = params.as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
        let w = (*ptr).width;
        let h = (*ptr).height;
        (w.max(0) as u32, h.max(0) as u32)
    }
}

/// Reads audio sample rate and channel count from codec parameters.
fn audio_params_from_parameters(params: &ffmpeg_next::codec::Parameters) -> (u32, u32) {
    unsafe {
        let ptr = params.as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
        let sr = (*ptr).sample_rate;
        let ch = (*ptr).ch_layout.nb_channels;
        (sr.max(0) as u32, ch.max(0) as u32)
    }
}
