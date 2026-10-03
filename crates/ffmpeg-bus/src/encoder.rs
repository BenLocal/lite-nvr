use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ffmpeg_next::{Dictionary, Rational, Rescale};
use tokio_util::sync::CancellationToken;

use crate::{
    frame::{RawFrame, RawFrameCmd, RawFrameReceiver},
    hw,
    packet::{RawPacket, RawPacketCmd, RawPacketReceiver, RawPacketSender},
    scaler::Scaler,
    stream::AvStream,
};

#[derive(Debug, Clone)]
pub struct AudioSettings {
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bitrate: Option<u64>,
    pub sample_format: Option<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            codec: Some("aac".to_string()),
            sample_rate: None,
            channels: None,
            bitrate: None,
            sample_format: None,
        }
    }
}

pub enum EncoderType {
    Video(ffmpeg_next::codec::encoder::Video),
    Audio(ffmpeg_next::codec::encoder::Audio),
}

impl EncoderType {
    pub fn send_frame(&mut self, frame: RawFrame, frame_index: i64) -> anyhow::Result<()> {
        match (self, frame) {
            (EncoderType::Video(encoder), RawFrame::Video(mut frame)) => {
                // Only the PTS may change: never copy the (shared) pixels.
                let frame = frame.props_mut();
                // Keyframe cadence comes from the encoder GOP (`Settings::keyframe_interval`).
                // Set PTS if not already set
                if frame.pts().is_none() {
                    frame.set_pts(Some(frame_index));
                }
                encoder.send_frame(frame)?;
            }
            (EncoderType::Audio(encoder), RawFrame::Audio(frame)) => {
                encoder.send_frame(frame.as_audio())?;
            }
            _ => anyhow::bail!("invalid frame type"),
        };

        Ok(())
    }

    pub fn send_eof(&mut self) -> anyhow::Result<()> {
        match self {
            EncoderType::Video(encoder) => encoder.send_eof()?,
            EncoderType::Audio(encoder) => encoder.send_eof()?,
        }
        Ok(())
    }

    pub fn encoder_receive_packet(
        &mut self,
        time_base: Rational,
    ) -> anyhow::Result<Option<RawPacket>> {
        let mut packet = ffmpeg_next::codec::packet::Packet::empty();
        let encode_result = match self {
            EncoderType::Video(encoder) => encoder.receive_packet(&mut packet),
            EncoderType::Audio(encoder) => encoder.receive_packet(&mut packet),
        };

        match encode_result {
            Ok(()) => Ok(Some(RawPacket::from((packet, time_base)))),
            Err(ffmpeg_next::Error::Other { errno })
                if errno == ffmpeg_next::util::error::EAGAIN =>
            {
                Ok(None)
            }
            Err(ffmpeg_next::Error::Eof) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    /// Frames between keyframes (GOP). `0` = automatic: about
    /// [`AUTO_GOP_SECS`] of video at the stream's frame rate.
    pub keyframe_interval: u64,
    pub codec: Option<String>,
    pub pixel_format: ffmpeg_next::format::Pixel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            keyframe_interval: 0,
            codec: Some("h264".to_string()),
            pixel_format: ffmpeg_next::format::Pixel::YUV420P,
        }
    }
}

/// Keyframe spacing used when [`Settings::keyframe_interval`] is `0`: a new
/// viewer of a live stream waits at most this long for a decodable picture.
pub const AUTO_GOP_SECS: u32 = 2;
/// Automatic GOP when the stream's frame rate is unknown.
const FALLBACK_GOP: u32 = 50;

/// GOP length in frames: the explicit interval, or [`AUTO_GOP_SECS`] worth of
/// frames at `rate` (a fixed frame count gives very different keyframe
/// spacing at 5fps vs 60fps).
fn gop_frames(keyframe_interval: u64, rate: Rational) -> u32 {
    if keyframe_interval > 0 {
        return u32::try_from(keyframe_interval).unwrap_or(u32::MAX);
    }
    if rate.numerator() <= 0 || rate.denominator() <= 0 {
        return FALLBACK_GOP;
    }
    let fps = f64::from(rate.numerator()) / f64::from(rate.denominator());
    ((fps * f64::from(AUTO_GOP_SECS)).round() as u32).max(1)
}

/// `requested` if the encoder supports it (or lists no constraint), else the
/// closest supported rate (the higher one on a tie).
fn pick_sample_rate(requested: u32, supported: &[u32]) -> u32 {
    if supported.is_empty() || supported.contains(&requested) {
        return requested;
    }
    supported
        .iter()
        .copied()
        .min_by_key(|&r| (r.abs_diff(requested), std::cmp::Reverse(r)))
        .unwrap_or(requested)
}

/// Returns a pixel format suitable for libx264. Source formats not supported by libx264 (e.g. rgb24)
/// are mapped to YUV420P; the encoder will use its internal scaler to convert when sending frames.
pub fn pixel_format_for_libx264(source: ffmpeg_next::format::Pixel) -> ffmpeg_next::format::Pixel {
    use ffmpeg_next::format::Pixel;
    match source {
        Pixel::RGB24 | Pixel::BGR24 => Pixel::YUV420P,
        _ => source,
    }
}

/// Resamples decoded audio to the encoder's sample format / rate / channel
/// layout and reframes it into fixed-size frames (the encoder's `frame_size`,
/// e.g. AAC's 1024 samples) via an `AVAudioFifo`. Codecs with a fixed frame
/// size reject arbitrarily-sized decoded frames, so a FIFO between the
/// resampler and the encoder is required.
struct AudioResampler {
    swr: ffmpeg_next::software::resampling::Context,
    fifo: *mut ffmpeg_next::ffi::AVAudioFifo,
    out_format: ffmpeg_next::format::Sample,
    out_layout: ffmpeg_next::ChannelLayout,
    out_rate: u32,
    /// Input parameters the current `swr` was built for.
    in_rate: u32,
    in_format: ffmpeg_next::format::Sample,
    in_layout: ffmpeg_next::ChannelLayout,
    frame_size: usize,
    /// Running output sample count, used as each emitted frame's PTS (in the
    /// encoder's `1/sample_rate` time base). Anchored on the first frame to the
    /// source's presentation time so copied video and transcoded audio stay in
    /// sync even when the stream does not start at PTS 0.
    next_pts: i64,
    started: bool,
    /// Input PTS (1/in_rate) the next frame should carry if nothing was lost.
    /// A later PTS means a gap (e.g. frames dropped on a live source).
    expected_in_pts: Option<i64>,
}

/// An input PTS jump beyond this (seconds) is a gap to bridge, not jitter.
const AUDIO_GAP_MIN_SECS: f64 = 0.05;
/// Gaps up to this long are filled with silence; longer jumps are treated as a
/// timestamp discontinuity and only move the output timeline forward.
const AUDIO_GAP_MAX_SILENCE_SECS: f64 = 2.0;

// The AVAudioFifo pointer is created, used, and freed only within this struct,
// which is not shared across threads concurrently.
unsafe impl Send for AudioResampler {}

impl AudioResampler {
    fn new(
        input: &ffmpeg_next::frame::Audio,
        out_rate: u32,
        out_format: ffmpeg_next::format::Sample,
        out_layout: ffmpeg_next::ChannelLayout,
        frame_size: u32,
    ) -> anyhow::Result<Self> {
        let swr = ffmpeg_next::software::resampling::Context::get(
            input.format(),
            input.channel_layout(),
            input.rate(),
            out_format,
            out_layout,
            out_rate,
        )?;
        let channels = out_layout.channels().max(1);
        let sample_fmt: ffmpeg_next::ffi::AVSampleFormat = out_format.into();
        let fifo = unsafe { ffmpeg_next::ffi::av_audio_fifo_alloc(sample_fmt, channels, 1) };
        if fifo.is_null() {
            anyhow::bail!("av_audio_fifo_alloc failed");
        }
        // frame_size == 0 means the codec accepts any frame size; pick a
        // reasonable default chunk.
        let frame_size = if frame_size == 0 {
            1024
        } else {
            frame_size as usize
        };
        Ok(Self {
            swr,
            fifo,
            out_format,
            out_layout,
            out_rate,
            in_rate: input.rate(),
            in_format: input.format(),
            in_layout: input.channel_layout(),
            frame_size,
            next_pts: 0,
            started: false,
            expected_in_pts: None,
        })
    }

    /// The source changed sample rate / format / layout mid-stream (swr would
    /// reject every frame from then on): flush what the old converter holds
    /// into the FIFO and rebuild it for the new input. The output timeline
    /// (FIFO + PTS) carries on unchanged.
    fn reconfigure(&mut self, input: &ffmpeg_next::frame::Audio) -> anyhow::Result<()> {
        log::info!(
            "audio input changed ({:?} {}Hz -> {:?} {}Hz): rebuilding resampler",
            self.in_format,
            self.in_rate,
            input.format(),
            input.rate()
        );
        loop {
            let mut tail =
                ffmpeg_next::frame::Audio::new(self.out_format, self.frame_size, self.out_layout);
            let more = self.swr.flush(&mut tail)?.is_some();
            if tail.samples() > 0 {
                self.fifo_write(&tail)?;
            }
            if !more || tail.samples() == 0 {
                break;
            }
        }
        self.swr = ffmpeg_next::software::resampling::Context::get(
            input.format(),
            input.channel_layout(),
            input.rate(),
            self.out_format,
            self.out_layout,
            self.out_rate,
        )?;
        self.in_rate = input.rate();
        self.in_format = input.format();
        self.in_layout = input.channel_layout();
        // Input PTS may now count in different units: no gap detection across
        // the switch.
        self.expected_in_pts = None;
        Ok(())
    }

    /// Resample `input` and buffer the converted samples in the FIFO.
    fn push(&mut self, input: &ffmpeg_next::frame::Audio) -> anyhow::Result<()> {
        if self.started
            && (input.rate() != self.in_rate
                || input.format() != self.in_format
                || input.channel_layout() != self.in_layout)
        {
            self.reconfigure(input)?;
        }
        if !self.started {
            self.started = true;
            // Anchor the output PTS to the first frame's presentation time so
            // audio stays aligned with copied video when the source starts off
            // zero. Decoded audio frame PTS are in 1/in_rate; rescale to out_rate.
            if let Some(p) = input.pts()
                && self.in_rate > 0
            {
                self.next_pts = (p as i128 * self.out_rate as i128 / self.in_rate as i128) as i64;
            }
        }
        if let Some(p) = input.pts() {
            if let Some(expected) = self.expected_in_pts {
                self.bridge_gap(p - expected)?;
            }
            self.expected_in_pts = Some(p + input.samples() as i64);
        }
        let max_out = unsafe {
            ffmpeg_next::ffi::swr_get_out_samples(self.swr.as_mut_ptr(), input.samples() as i32)
        };
        if max_out <= 0 {
            return Ok(());
        }
        let mut converted =
            ffmpeg_next::frame::Audio::new(self.out_format, max_out as usize, self.out_layout);
        self.swr.run(input, &mut converted)?;
        self.fifo_write(&converted)?;
        Ok(())
    }

    /// Keep the output timeline aligned with the source across lost input
    /// (`gap` input samples missing). Without this, PTS derived from the
    /// sample count would run ahead of copied video after every drop.
    fn bridge_gap(&mut self, gap: i64) -> anyhow::Result<()> {
        if self.in_rate == 0 {
            return Ok(());
        }
        let secs = gap as f64 / self.in_rate as f64;
        if secs <= AUDIO_GAP_MIN_SECS {
            return Ok(());
        }
        let out_samples = (gap as i128 * self.out_rate as i128 / self.in_rate as i128) as i64;
        if secs <= AUDIO_GAP_MAX_SILENCE_SECS {
            self.fifo_write_silence(out_samples as usize)
        } else {
            log::warn!("audio timestamp jump of {secs:.1}s: moving the output timeline");
            self.next_pts += out_samples;
            Ok(())
        }
    }

    fn fifo_write_silence(&mut self, samples: usize) -> anyhow::Result<()> {
        if samples == 0 {
            return Ok(());
        }
        let mut silence = ffmpeg_next::frame::Audio::new(self.out_format, samples, self.out_layout);
        let channels = self.out_layout.channels().max(1);
        let fmt: ffmpeg_next::ffi::AVSampleFormat = self.out_format.into();
        // SAFETY: `silence` was just allocated for `samples` samples of
        // `out_format` x `channels`, and extended_data has one plane per
        // channel (planar) or one interleaved plane, as av_samples_set_silence
        // expects.
        unsafe {
            ffmpeg_next::ffi::av_samples_set_silence(
                (*silence.as_mut_ptr()).extended_data,
                0,
                samples as i32,
                channels,
                fmt,
            );
        }
        self.fifo_write(&silence)
    }

    /// Pull every full `frame_size` frame currently buffered.
    fn drain(&mut self) -> anyhow::Result<Vec<ffmpeg_next::frame::Audio>> {
        let mut out = Vec::new();
        while self.fifo_size() >= self.frame_size as i32 {
            out.push(self.read_frame(self.frame_size)?);
        }
        Ok(out)
    }

    /// Flush the resampler's internal buffer, then emit all remaining frames
    /// including a final short one. Call once at end of stream.
    fn flush(&mut self) -> anyhow::Result<Vec<ffmpeg_next::frame::Audio>> {
        loop {
            let mut converted =
                ffmpeg_next::frame::Audio::new(self.out_format, self.frame_size, self.out_layout);
            let more = self.swr.flush(&mut converted)?.is_some();
            if converted.samples() > 0 {
                self.fifo_write(&converted)?;
            }
            if !more || converted.samples() == 0 {
                break;
            }
        }
        let mut out = self.drain()?;
        let rem = self.fifo_size();
        if rem > 0 {
            out.push(self.read_frame(rem as usize)?);
        }
        Ok(out)
    }

    fn fifo_size(&self) -> i32 {
        unsafe { ffmpeg_next::ffi::av_audio_fifo_size(self.fifo) }
    }

    fn fifo_write(&mut self, frame: &ffmpeg_next::frame::Audio) -> anyhow::Result<()> {
        let n = frame.samples();
        if n == 0 {
            return Ok(());
        }
        let written = unsafe {
            ffmpeg_next::ffi::av_audio_fifo_write(
                self.fifo,
                (*frame.as_ptr()).extended_data as *const *mut std::ffi::c_void,
                n as i32,
            )
        };
        if written < n as i32 {
            anyhow::bail!("av_audio_fifo_write short write: {} < {}", written, n);
        }
        Ok(())
    }

    fn read_frame(&mut self, n: usize) -> anyhow::Result<ffmpeg_next::frame::Audio> {
        let mut frame = ffmpeg_next::frame::Audio::new(self.out_format, n, self.out_layout);
        let got = unsafe {
            ffmpeg_next::ffi::av_audio_fifo_read(
                self.fifo,
                (*frame.as_mut_ptr()).extended_data as *const *mut std::ffi::c_void,
                n as i32,
            )
        };
        if got < 0 {
            anyhow::bail!("av_audio_fifo_read failed");
        }
        frame.set_samples(got as usize);
        frame.set_rate(self.out_rate);
        frame.set_pts(Some(self.next_pts));
        self.next_pts += got as i64;
        Ok(frame)
    }
}

impl Drop for AudioResampler {
    fn drop(&mut self) {
        unsafe { ffmpeg_next::ffi::av_audio_fifo_free(self.fifo) };
    }
}

pub struct Encoder {
    stream: AvStream,
    inner: EncoderType,
    encoder_time_base: Rational,
    interleaved: bool,
    frame_index: i64,
    scaler: Option<Scaler>,
    /// Input (format, width, height) the current `scaler` was built for.
    scaler_input: Option<(ffmpeg_next::format::Pixel, u32, u32)>,
    audio_resampler: Option<AudioResampler>,
    /// Encoding on a hardware codec; cleared after a runtime downgrade.
    is_hw: bool,
    /// Video settings + options to reopen on a software codec (see
    /// [`Encoder::downgrade_to_software`]). Options kept as strings:
    /// `Dictionary` is not `Send`.
    reopen: Option<(Settings, Vec<(String, String)>)>,
    /// Name of the selected codec (to blacklist a failing hardware one).
    codec_name: String,
    /// Test hook: fail the next frame as a broken hardware encoder would.
    #[cfg(test)]
    inject_hw_failure: bool,
}

impl Encoder {
    fn open_video_encoder_with_codec(
        stream: &AvStream,
        codec: ffmpeg_next::Codec,
        settings: &Settings,
        options: Option<Dictionary>,
    ) -> anyhow::Result<(ffmpeg_next::codec::encoder::Video, Rational)> {
        let encoder_context = ffmpeg_next::codec::Context::new_with_codec(codec);
        let mut encoder = encoder_context.encoder().video()?;
        encoder.set_width(settings.width);
        encoder.set_height(settings.height);
        encoder.set_format(settings.pixel_format);
        encoder.set_frame_rate(Some(stream.rate()));
        encoder.set_time_base(ffmpeg_next::util::mathematics::rescale::TIME_BASE);
        // Bounded GOP so live viewers can start decoding within one interval.
        encoder.set_gop(gop_frames(settings.keyframe_interval, stream.rate()));

        let need_defaults = options.is_none();
        let mut opts = options.unwrap_or_default();
        if need_defaults {
            opts.set("preset", "ultrafast");
            opts.set("tune", "zerolatency");
        }
        let encoder = encoder.open_with(opts)?;
        let encoder_time_base: Rational = unsafe { (*encoder.0.as_ptr()).time_base.into() };
        Ok((encoder, encoder_time_base))
    }

    pub fn new(
        stream: &AvStream,
        settings: Settings,
        options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        let options_kv: Vec<(String, String)> = options
            .as_ref()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let requested = settings.codec.as_deref();
        let candidates = hw::video_encoder_candidates(requested);
        let mut selected_name: Option<String> = None;
        let mut selected_is_hw = false;
        let mut first_hw_failure: Option<String> = None;
        let mut opened: Option<(ffmpeg_next::codec::encoder::Video, Rational)> = None;

        for candidate in candidates {
            let Some(codec) = ffmpeg_next::encoder::find_by_name(&candidate.name) else {
                continue;
            };
            match Self::open_video_encoder_with_codec(stream, codec, &settings, options.clone()) {
                Ok(v) => {
                    selected_name = Some(candidate.name.clone());
                    selected_is_hw = candidate.is_hw;
                    opened = Some(v);
                    break;
                }
                Err(e) => {
                    if candidate.is_hw && first_hw_failure.is_none() {
                        first_hw_failure = Some(format!("{} open failed: {}", candidate.name, e));
                    }
                    log::info!(
                        "video encoder candidate rejected: name={}, hw={}, reason={}",
                        candidate.name,
                        candidate.is_hw,
                        e
                    );
                }
            }
        }

        let (encoder, encoder_time_base) = opened.ok_or_else(|| {
            anyhow::anyhow!(
                "no usable video encoder for requested codec {:?}",
                settings.codec
            )
        })?;
        if selected_is_hw {
            log::info!(
                "video encoder selected: {} (hardware), stream_index={}",
                selected_name.as_deref().unwrap_or("unknown"),
                stream.index()
            );
        } else {
            if let Some(reason) = first_hw_failure {
                log::info!(
                    "hardware encode unavailable, fallback to software: {}",
                    reason
                );
            } else {
                log::info!("video encoder selected: software fallback");
            }
            log::info!(
                "video encoder selected: {} (software), stream_index={}",
                selected_name.as_deref().unwrap_or("unknown"),
                stream.index()
            );
        }

        Ok(Self {
            stream: stream.clone(),
            inner: EncoderType::Video(encoder),
            encoder_time_base: encoder_time_base,
            interleaved: false,
            frame_index: 0,
            scaler: None,
            scaler_input: None,
            audio_resampler: None,
            is_hw: selected_is_hw,
            codec_name: selected_name.clone().unwrap_or_default(),
            reopen: Some((settings, options_kv)),
            #[cfg(test)]
            inject_hw_failure: false,
        })
    }

    pub fn new_audio(
        stream: &AvStream,
        settings: AudioSettings,
        options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        let codec_name = settings.codec.as_deref().unwrap_or("aac");
        let codec = ffmpeg_next::encoder::find_by_name(codec_name)
            .ok_or_else(|| anyhow::anyhow!("audio encoder not found: {}", codec_name))?;

        let encoder_context = ffmpeg_next::codec::Context::new_with_codec(codec);
        let mut encoder = encoder_context.encoder().audio()?;

        // Use settings or fall back to input stream parameters
        let sample_rate = settings.sample_rate.unwrap_or_else(|| unsafe {
            let ptr = stream.parameters().as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
            (*ptr).sample_rate.max(0) as u32
        });
        let sample_rate = if sample_rate == 0 { 44100 } else { sample_rate };
        // Some encoders only take specific rates (libopus: 48k/24k/16k/12k/8k);
        // the resampler converts the input to whatever is picked here.
        let supported: Vec<u32> = codec
            .audio()
            .ok()
            .and_then(|a| a.rates())
            .map(|rates| rates.filter_map(|r| u32::try_from(r).ok()).collect())
            .unwrap_or_default();
        let picked = pick_sample_rate(sample_rate, &supported);
        if picked != sample_rate {
            log::info!("{codec_name}: sample rate {sample_rate} unsupported, using {picked}");
        }
        let sample_rate = picked;
        encoder.set_rate(sample_rate as i32);

        // Set channel layout
        let channels = settings.channels.unwrap_or_else(|| unsafe {
            let ptr = stream.parameters().as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
            let ch = ffmpeg_next::ffi::AVChannelLayout { ..(*ptr).ch_layout };
            ch.nb_channels.max(0) as u32
        });
        let channels = if channels == 0 { 2 } else { channels };
        unsafe {
            ffmpeg_next::ffi::av_channel_layout_default(
                &mut (*encoder.as_mut_ptr()).ch_layout,
                channels as i32,
            );
        }

        // Set sample format
        if let Some(ref fmt_name) = settings.sample_format {
            let av_fmt: ffmpeg_next::ffi::AVSampleFormat = unsafe {
                ffmpeg_next::ffi::av_get_sample_fmt(
                    std::ffi::CString::new(fmt_name.as_str()).unwrap().as_ptr(),
                )
            };
            let fmt: ffmpeg_next::format::Sample = av_fmt.into();
            encoder.set_format(fmt);
        } else {
            // Use first supported format from codec, or default to FLTP
            let default_fmt = unsafe {
                let codec_ptr = codec.as_ptr();
                let sample_fmts = (*codec_ptr).sample_fmts;
                if !sample_fmts.is_null() {
                    (*sample_fmts).into()
                } else {
                    ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar)
                }
            };
            encoder.set_format(default_fmt);
        }

        // Audio encoders use a 1/sample_rate time base. Frame PTS are emitted as
        // a running output-sample count (see AudioResampler), so this makes the
        // muxer rescale audio timestamps correctly and keeps A/V in sync.
        encoder.set_time_base(Rational(1, sample_rate as i32));

        if let Some(bitrate) = settings.bitrate {
            encoder.set_bit_rate(bitrate as usize);
        }

        let encoder = if let Some(opts) = options {
            encoder.open_with(opts)?
        } else {
            encoder.open_with(Dictionary::new())?
        };
        let encoder_time_base: Rational = unsafe { (*encoder.0.as_ptr()).time_base.into() };

        log::info!(
            "audio encoder selected: {} (software), stream_index={}, sample_rate={}, channels={}",
            codec_name,
            stream.index(),
            sample_rate,
            channels,
        );

        Ok(Self {
            stream: stream.clone(),
            inner: EncoderType::Audio(encoder),
            encoder_time_base,
            interleaved: false,
            frame_index: 0,
            scaler: None,
            scaler_input: None,
            audio_resampler: None,
            is_hw: false,
            codec_name: codec_name.to_string(),
            reopen: None,
            #[cfg(test)]
            inject_hw_failure: false,
        })
    }

    pub fn send_frame(&mut self, mut frame: RawFrame) -> anyhow::Result<()> {
        // What to hand the encoder: either the input frame unchanged, or a set
        // of derived frames (a scaled video frame, or resampled/reframed audio
        // frames). Computed while borrowing `frame`, then acted on afterwards so
        // the original frame can be moved into the copy path.
        enum Outbound {
            Original,
            Frames(Vec<RawFrame>),
        }

        let action = match &mut frame {
            RawFrame::Video(vf) => {
                // Decoded and raw-video frames carry PTS in the input stream's
                // time base; the encoder runs in its own (1/1_000_000), and its
                // packets are tagged with that, so convert before encoding or
                // the output plays back at the wrong speed.
                let src_tb = self.stream.time_base();
                if src_tb.numerator() > 0 && src_tb != self.encoder_time_base {
                    let f = vf.props_mut();
                    if let Some(pts) = f.pts() {
                        f.set_pts(Some(pts.rescale(src_tb, self.encoder_time_base)));
                    }
                }
                let (ef, ew, eh) = match &self.inner {
                    EncoderType::Video(e) => (e.format(), e.width(), e.height()),
                    _ => anyhow::bail!("video frame sent to non-video encoder"),
                };
                // Read-only: the scaler writes into a new frame.
                let f = vf.as_video();
                if f.format() != ef || f.width() != ew || f.height() != eh {
                    // (Re)build on first use and whenever the source changes
                    // resolution / format mid-stream: a stale scaler rejects
                    // every frame (InputChanged) and the output goes dark.
                    let input_key = (f.format(), f.width(), f.height());
                    if self.scaler_input != Some(input_key) {
                        self.scaler = None;
                        self.scaler_input = Some(input_key);
                    }
                    if self.scaler.is_none() {
                        self.scaler =
                            Some(Scaler::new(ffmpeg_next::software::scaling::Context::get(
                                f.format(),
                                f.width(),
                                f.height(),
                                ef,
                                ew,
                                eh,
                                ffmpeg_next::software::scaling::flag::Flags::empty(),
                            )?));
                    }

                    let mut converted = ffmpeg_next::frame::Video::empty();
                    self.scaler.as_mut().unwrap().run(f, &mut converted)?;
                    // Copy over PTS from old frame.
                    converted.set_pts(f.pts());
                    Outbound::Frames(vec![RawFrame::Video(converted.into())])
                } else {
                    Outbound::Original
                }
            }
            RawFrame::Audio(af) => {
                let (rate, fmt, layout, frame_size) = match &self.inner {
                    EncoderType::Audio(e) => {
                        (e.rate(), e.format(), e.channel_layout(), e.frame_size())
                    }
                    _ => anyhow::bail!("audio frame sent to non-audio encoder"),
                };
                let in_af = af.as_audio();
                if self.audio_resampler.is_none() {
                    self.audio_resampler =
                        Some(AudioResampler::new(in_af, rate, fmt, layout, frame_size)?);
                }
                let resampler = self.audio_resampler.as_mut().unwrap();
                resampler.push(in_af)?;
                let chunks = resampler.drain()?;
                Outbound::Frames(
                    chunks
                        .into_iter()
                        .map(|c| RawFrame::Audio(c.into()))
                        .collect(),
                )
            }
        };

        match action {
            Outbound::Original => self.send_to_inner(frame)?,
            Outbound::Frames(frames) => {
                for f in frames {
                    self.send_to_inner(f)?;
                }
            }
        }
        Ok(())
    }

    /// Hand one frame to the codec. A hardware encoder can open cleanly yet
    /// fail on real frames (as QSV decoders do here); then downgrade to
    /// software once and replay the frame, instead of producing nothing.
    fn send_to_inner(&mut self, frame: RawFrame) -> anyhow::Result<()> {
        // Arc clone (no pixel copy), only needed while on hardware.
        let retry = self.is_hw.then(|| frame.clone());
        #[cfg(test)]
        let injected = std::mem::take(&mut self.inject_hw_failure);
        #[cfg(not(test))]
        let injected = false;
        let result = if injected {
            Err(anyhow::anyhow!("injected hardware encoder failure"))
        } else {
            self.inner.send_frame(frame, self.frame_index)
        };
        match (result, retry) {
            (Ok(()), _) => {}
            (Err(e), Some(frame)) => {
                log::warn!(
                    "stream {}: hardware encode failed at runtime ({e:#}); \
                     falling back to software encoder",
                    self.stream.index()
                );
                self.downgrade_to_software()?;
                self.inner.send_frame(frame, self.frame_index)?;
            }
            (Err(e), None) => return Err(e),
        }
        self.frame_index += 1;
        Ok(())
    }

    /// Replace the hardware video codec with the first software candidate for
    /// the same settings. Frames still inside the hardware codec are lost.
    fn downgrade_to_software(&mut self) -> anyhow::Result<()> {
        hw::mark_runtime_failure(&self.codec_name);
        let (settings, options) = self
            .reopen
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no settings to reopen the encoder with"))?;
        let mut last_err = None;
        for candidate in hw::video_encoder_candidates(settings.codec.as_deref())
            .into_iter()
            .filter(|c| !c.is_hw)
        {
            let Some(codec) = ffmpeg_next::encoder::find_by_name(&candidate.name) else {
                continue;
            };
            let opts = (!options.is_empty()).then(|| {
                Dictionary::from_iter(options.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            });
            match Self::open_video_encoder_with_codec(&self.stream, codec, &settings, opts) {
                Ok((encoder, time_base)) => {
                    log::info!(
                        "video encoder selected: {} (software, after hardware failure), stream_index={}",
                        candidate.name,
                        self.stream.index()
                    );
                    self.inner = EncoderType::Video(encoder);
                    self.encoder_time_base = time_base;
                    self.is_hw = false;
                    self.codec_name = candidate.name;
                    return Ok(());
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no software video encoder available")))
    }

    pub fn send_eof(&mut self) -> anyhow::Result<()> {
        // Flush the audio resampler's buffered/tail samples before EOF so no
        // audio is dropped at end of stream.
        let chunks = if let Some(resampler) = self.audio_resampler.as_mut() {
            resampler.flush()?
        } else {
            Vec::new()
        };
        for chunk in chunks {
            self.inner
                .send_frame(RawFrame::Audio(chunk.into()), self.frame_index)?;
            self.frame_index += 1;
        }
        self.inner.send_eof()
    }

    /// Describe this encoder's output stream for muxing, taking the codec
    /// parameters (sample rate / channels / dimensions and the encoder-generated
    /// extradata) from the encoder context rather than the input stream. A
    /// param-changing transcode (e.g. 44100/mono -> 48000/stereo) must advertise
    /// the *encoder's* params, or the muxed header won't match the packets.
    /// `index` keys the muxer's input->output stream mapping.
    pub fn output_stream(&self, index: usize) -> AvStream {
        let params = match &self.inner {
            EncoderType::Video(e) => ffmpeg_next::codec::Parameters::from(e),
            EncoderType::Audio(e) => ffmpeg_next::codec::Parameters::from(e),
        };
        AvStream::new(index, params, self.encoder_time_base, self.stream.rate())
    }

    pub fn encoder_receive_packet(&mut self) -> anyhow::Result<Option<RawPacket>> {
        let mut pkt = match self.inner.encoder_receive_packet(self.encoder_time_base) {
            Ok(pkt) => pkt,
            // Asynchronous hardware codecs may report failures here instead of
            // in send_frame: downgrade the same way.
            Err(e) if self.is_hw => {
                log::warn!(
                    "stream {}: hardware encode failed at runtime ({e:#}); \
                     falling back to software encoder",
                    self.stream.index()
                );
                self.downgrade_to_software()?;
                None
            }
            Err(e) => return Err(e),
        };

        if let Some(ref mut p) = pkt {
            match &self.inner {
                EncoderType::Video(_) => {
                    let rate = self.stream.rate();
                    if rate.0 > 0 {
                        let duration = 1_000_000i64 * rate.1 as i64 / rate.0 as i64;
                        p.set_duration(duration);
                    }
                }
                EncoderType::Audio(encoder) => {
                    let frame_size = encoder.frame_size() as i64;
                    let rate = encoder.rate() as i64;
                    let tb = self.encoder_time_base;
                    if frame_size > 0 && rate > 0 && tb.0 != 0 {
                        // Duration of `frame_size` samples expressed in the
                        // encoder time base: frame_size/rate seconds ÷ (num/den).
                        let duration = frame_size * tb.1 as i64 / (rate * tb.0 as i64);
                        p.set_duration(duration);
                    }
                }
            }
        }
        Ok(pkt)
    }
}

/// Encoder output = encoded packets (small). Moderate capacity for bursts.
/// Also the backpressure high-water mark for a lossless encoder's output.
const PACKET_CHAN_CAP: usize = 64;

/// Subscription bookkeeping shared between [`EncoderTask::subscribe`] and the
/// encode loop (see [`EncoderTask::new_auto_stop`]).
#[derive(Default)]
struct OutputState {
    /// Someone subscribed at least once (auto-stop only after that).
    had_any: bool,
    /// The encoder has ended or stopped: no further subscriptions.
    closed: bool,
}

type SharedOutputState = Arc<Mutex<OutputState>>;

fn lock_state(state: &SharedOutputState) -> std::sync::MutexGuard<'_, OutputState> {
    // Only flag updates happen under this lock, so a poisoned lock is still
    // consistent: recover instead of panicking.
    state.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct EncoderTask {
    cancel: CancellationToken,
    raw_chan: RawPacketSender,
    state: SharedOutputState,
    /// Stop once the last output subscriber leaves (bus-managed encoders).
    auto_stop: bool,
}

impl EncoderTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        let (sender, _) = tokio::sync::broadcast::channel(PACKET_CHAN_CAP);

        Self {
            cancel,
            raw_chan: sender,
            state: SharedOutputState::default(),
            auto_stop: false,
        }
    }

    /// An encoder that stops itself when its last output subscriber goes
    /// away, so a shared encoder nobody reads any more stops encoding.
    pub(crate) fn new_auto_stop() -> Self {
        let mut task = Self::new();
        task.auto_stop = true;
        task
    }

    /// Subscribe to encoded packets. On an encoder that has ended, the
    /// receiver reports `Closed` at once instead of waiting forever.
    pub fn subscribe(&self) -> RawPacketReceiver {
        self.try_subscribe()
            .unwrap_or_else(|| tokio::sync::broadcast::channel(1).1)
    }

    /// Like [`Self::subscribe`], but `None` if the encoder has ended, so the
    /// caller can start a fresh one.
    pub(crate) fn try_subscribe(&self) -> Option<RawPacketReceiver> {
        let mut state = lock_state(&self.state);
        if state.closed {
            return None;
        }
        state.had_any = true;
        Some(self.raw_chan.subscribe())
    }

    /// Still encoding and accepting subscribers.
    pub(crate) fn is_running(&self) -> bool {
        !self.cancel.is_cancelled() && !lock_state(&self.state).closed
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub async fn start(
        &self,
        encoder: Encoder,
        mut encoder_receiver: RawFrameReceiver,
        lossless: bool,
    ) {
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();
        let state = self.state.clone();
        let auto_stop = self.auto_stop;
        log::info!(
            "encoder loop started, stream index: {}, lossless: {}",
            encoder.stream.index(),
            lossless
        );
        /// Bounded queue: when encoder is slower than producer, back-pressure instead of unbounded growth (OOM).
        const FRAME_QUEUE_BOUND: usize = 128;
        /// Log "queue full" at most every N drops; use debug level so info logs stay clean.
        const DROP_LOG_INTERVAL: u64 = 120;
        tokio::spawn(async move {
            let (tx, rx) = std::sync::mpsc::sync_channel::<RawFrameCmd>(FRAME_QUEUE_BOUND);
            let handle_cancel = cancel_clone.clone();
            let handle = tokio::task::spawn_blocking(move || {
                Self::encoder_loop(
                    encoder,
                    handle_cancel,
                    rx,
                    sender_clone,
                    lossless,
                    (state, auto_stop),
                )
            });
            let mut dropped_count: u64 = 0;
            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        break;
                    }
                    result = encoder_receiver.recv() => {
                    match result {
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::debug!("encoder relay: lagged, lost {} frames", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                    Ok(frame) => {
                        let is_eof = matches!(&frame, RawFrameCmd::EOF);
                        // EOF must always land; lossless mode (file/net transcode)
                        // backpressures every frame so none are dropped. Lossy
                        // mode (live) drops DATA when the queue is full to bound
                        // latency/memory.
                        let disconnected = if is_eof || lossless {
                            Self::relay_send_backpressure(&tx, &cancel_clone, frame).await
                        } else {
                            match tx.try_send(frame) {
                                Ok(()) => false,
                                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                                    dropped_count += 1;
                                    if dropped_count % DROP_LOG_INTERVAL == 1 {
                                        log::debug!(
                                            "encoder frame queue full, dropped {} frames (back-pressure)",
                                            dropped_count
                                        );
                                    }
                                    false
                                }
                                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => true,
                            }
                        };
                        if disconnected {
                            break;
                        }
                    }
                    }
                    }
                }
            }
            let _ = handle.await;
            log::info!("encoder task finished");
        });
    }

    /// Send a frame into the bounded encoder queue, waiting (async, so the
    /// executor stays free) for room instead of dropping. Returns true if the
    /// encoder loop's receiver has gone away, so the caller should stop.
    async fn relay_send_backpressure(
        tx: &std::sync::mpsc::SyncSender<RawFrameCmd>,
        cancel: &CancellationToken,
        frame: RawFrameCmd,
    ) -> bool {
        let mut pending = frame;
        loop {
            match tx.try_send(pending) {
                Ok(()) => return false,
                Err(std::sync::mpsc::TrySendError::Full(f)) => {
                    if cancel.is_cancelled() {
                        return false;
                    }
                    pending = f;
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return true,
            }
        }
    }

    fn encoder_loop(
        mut encoder: Encoder,
        cancel: CancellationToken,
        rx: std::sync::mpsc::Receiver<RawFrameCmd>,
        out: RawPacketSender,
        lossless: bool,
        (state, auto_stop): (SharedOutputState, bool),
    ) {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let mut eof = false;
            match rx.recv_timeout(Duration::from_millis(1)) {
                Ok(frame) => {
                    if auto_stop {
                        // Checked under the subscription lock, so a concurrent
                        // subscribe either lands first (keeping us alive) or
                        // sees `closed` and is refused.
                        let mut st = lock_state(&state);
                        if st.had_any && out.receiver_count() == 0 {
                            st.closed = true;
                            cancel.cancel();
                            break;
                        }
                    }
                    match frame {
                        RawFrameCmd::Data(frame) => {
                            if let Err(e) = encoder.send_frame(frame) {
                                log::error!("send packet error: {}", e);
                                continue;
                            }
                        }
                        RawFrameCmd::EOF => {
                            if let Err(e) = encoder.send_eof() {
                                log::error!("send eof error: {}", e);
                            }
                            eof = true;
                        }
                    };

                    'outer: loop {
                        match encoder.encoder_receive_packet() {
                            Ok(Some(packet)) => {
                                Self::send_packet_backpressure(
                                    &out,
                                    &cancel,
                                    lossless,
                                    RawPacketCmd::Data(packet),
                                );
                            }
                            Ok(None) => {
                                break 'outer;
                            }
                            Err(e) => {
                                log::error!("receive packet error: {}", e);
                                break 'outer;
                            }
                        }
                    }

                    if eof {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
                // Relay gone (decoder closed or task cancelled): stop instead
                // of spinning on a dead queue.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Self::send_packet_backpressure(&out, &cancel, lossless, RawPacketCmd::EOF);
        lock_state(&state).closed = true;
    }

    /// Publish an encoded packet. A lossless encoder (file/net transcode)
    /// waits for its muxer to catch up instead of overwriting unread packets.
    fn send_packet_backpressure(
        out: &RawPacketSender,
        cancel: &CancellationToken,
        lossless: bool,
        msg: RawPacketCmd,
    ) {
        if lossless {
            while out.len() >= PACKET_CHAN_CAP && out.receiver_count() > 0 && !cancel.is_cancelled()
            {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let _ = out.send(msg);
    }
}

impl Drop for EncoderTask {
    /// Dropping the task (bus teardown / input removal) stops the relay and the
    /// blocking encode loop, so they never outlive the bus.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
#[path = "encoder_test.rs"]
mod encoder_test;
