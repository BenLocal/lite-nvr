use std::time::Duration;

use ffmpeg_next::{Dictionary, Rational, picture};
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
                let frame = frame.get_mut();
                // todo
                if frame_index % 5 == 0 {
                    frame.set_kind(picture::Type::I);
                }
                // Set PTS if not already set
                if frame.pts().is_none() {
                    frame.set_pts(Some(frame_index));
                }
                encoder.send_frame(frame)?;
            }
            (EncoderType::Audio(encoder), RawFrame::Audio(mut frame)) => {
                let frame = frame.get_mut();
                encoder.send_frame(frame)?;
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
    pub keyframe_interval: u64,
    pub codec: Option<String>,
    pub pixel_format: ffmpeg_next::format::Pixel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            keyframe_interval: 25,
            codec: Some("h264".to_string()),
            pixel_format: ffmpeg_next::format::Pixel::YUV420P,
        }
    }
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
    in_rate: u32,
    frame_size: usize,
    /// Running output sample count, used as each emitted frame's PTS (in the
    /// encoder's `1/sample_rate` time base). Anchored on the first frame to the
    /// source's presentation time so copied video and transcoded audio stay in
    /// sync even when the stream does not start at PTS 0.
    next_pts: i64,
    started: bool,
}

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
            frame_size,
            next_pts: 0,
            started: false,
        })
    }

    /// Resample `input` and buffer the converted samples in the FIFO.
    fn push(&mut self, input: &ffmpeg_next::frame::Audio) -> anyhow::Result<()> {
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
    audio_resampler: Option<AudioResampler>,
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
                        first_hw_failure =
                            Some(format!("{} open failed: {}", candidate.name, e));
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
                log::info!("hardware encode unavailable, fallback to software: {}", reason);
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
            audio_resampler: None,
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
        let sample_rate = settings.sample_rate.unwrap_or_else(|| {
            unsafe {
                let ptr =
                    stream.parameters().as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
                (*ptr).sample_rate.max(0) as u32
            }
        });
        let sample_rate = if sample_rate == 0 { 44100 } else { sample_rate };
        encoder.set_rate(sample_rate as i32);

        // Set channel layout
        let channels = settings.channels.unwrap_or_else(|| {
            unsafe {
                let ptr =
                    stream.parameters().as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
                let ch = ffmpeg_next::ffi::AVChannelLayout {
                    ..(*ptr).ch_layout
                };
                ch.nb_channels.max(0) as u32
            }
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
                    std::ffi::CString::new(fmt_name.as_str())
                        .unwrap()
                        .as_ptr(),
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
            audio_resampler: None,
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
                let (ef, ew, eh) = match &self.inner {
                    EncoderType::Video(e) => (e.format(), e.width(), e.height()),
                    _ => anyhow::bail!("video frame sent to non-video encoder"),
                };
                let f = vf.get_mut();
                if f.format() != ef || f.width() != ew || f.height() != eh {
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
                let in_af = af.get_mut();
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
            Outbound::Original => {
                self.inner.send_frame(frame, self.frame_index)?;
                self.frame_index += 1;
            }
            Outbound::Frames(frames) => {
                for f in frames {
                    self.inner.send_frame(f, self.frame_index)?;
                    self.frame_index += 1;
                }
            }
        }
        Ok(())
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
        let mut pkt = self.inner.encoder_receive_packet(self.encoder_time_base)?;

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

pub struct EncoderTask {
    cancel: CancellationToken,
    raw_chan: RawPacketSender,
}

impl EncoderTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        /// Encoder output = encoded packets (small). Moderate capacity for bursts.
        const PACKET_CHAN_CAP: usize = 64;
        let (sender, _) = tokio::sync::broadcast::channel(PACKET_CHAN_CAP);

        Self {
            cancel,
            raw_chan: sender,
        }
    }

    pub fn subscribe(&self) -> RawPacketReceiver {
        self.raw_chan.subscribe()
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
                Self::encoder_loop(encoder, handle_cancel, rx, sender_clone)
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
    ) {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let mut eof = false;
            match rx.recv_timeout(Duration::from_millis(1)) {
                Ok(frame) => {
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
                                let _ = out.send(RawPacketCmd::Data(packet));
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
                Err(_) => (),
            }
        }
        let _ = out.send(RawPacketCmd::EOF);
    }
}
