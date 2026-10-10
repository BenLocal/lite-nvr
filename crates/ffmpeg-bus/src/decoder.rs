use std::{
    backtrace::Backtrace,
    sync::{Arc, Mutex},
    time::Duration,
};

use ffmpeg_next::Rational;
use tokio_util::sync::CancellationToken;

use crate::{
    frame::{
        RawAudioFrame, RawFrame, RawFrameCmd, RawFrameReceiver, RawFrameSender, RawVideoFrame,
    },
    hw,
    packet::{RawPacket, RawPacketCmd, RawPacketReceiver},
    stream::AvStream,
};

/// Per-subscriber decoded-frame ring-buffer size. Balances memory vs avoiding
/// Lagged (dropped frames break a stream). Used both to size each subscriber's
/// channel and as the backpressure high-water mark for lossless subscribers.
const FRAME_CHAN_CAP: usize = 16;

/// Send a decoded frame to one subscriber. A `lossless` subscriber (file/net
/// transcode) makes the decoder wait for room in *its own* ring buffer, so a
/// fast producer (e.g. a whole file decoded in a burst) does not overwrite
/// unconsumed frames. A lossy subscriber (live, raw, ASR, detection) never
/// blocks the decoder: when it lags, its own oldest frames are dropped.
fn send_frame_backpressure(
    sender: &RawFrameSender,
    cancel: &CancellationToken,
    lossless: bool,
    msg: RawFrameCmd,
) {
    if lossless {
        while sender.len() >= FRAME_CHAN_CAP
            && sender.receiver_count() > 0
            && !cancel.is_cancelled()
        {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let _ = sender.send(msg);
}

/// One consumer of a decoder's output, with its own channel and drop policy.
struct FrameSubscriber {
    tx: RawFrameSender,
    lossless: bool,
}

/// Fan-out state shared between [`DecoderTask::subscribe`] and the decode loop.
#[derive(Default)]
struct SubscriberState {
    subs: Vec<FrameSubscriber>,
    /// Someone subscribed at least once (auto-stop only after that).
    had_any: bool,
    /// The decoder has ended or stopped: no further subscriptions.
    closed: bool,
}

#[derive(Clone, Default)]
struct FrameSubscribers(Arc<Mutex<SubscriberState>>);

/// A receiver whose sender is already gone: yields `Closed` immediately, so a
/// subscriber of an ended decoder sees end-of-stream instead of hanging.
pub(crate) fn closed_receiver() -> RawFrameReceiver {
    tokio::sync::broadcast::channel(1).1
}

impl FrameSubscribers {
    /// `None` once the decoder has ended (see [`Self::close`]).
    fn add(&self, lossless: bool) -> Option<RawFrameReceiver> {
        let mut state = self.lock();
        if state.closed {
            return None;
        }
        let (tx, rx) = tokio::sync::broadcast::channel(FRAME_CHAN_CAP);
        state.subs.push(FrameSubscriber { tx, lossless });
        state.had_any = true;
        Some(rx)
    }

    /// Deliver `msg` to every live subscriber. Senders are snapshotted so the
    /// lock is not held while a lossless subscriber applies backpressure.
    /// With `auto_stop`, the last subscriber leaving closes the fan-out and
    /// cancels the decoder: nobody is left to use the frames.
    fn publish(&self, cancel: &CancellationToken, auto_stop: bool, msg: RawFrameCmd) {
        let targets: Vec<(RawFrameSender, bool)> = {
            let mut state = self.lock();
            state.subs.retain(|s| s.tx.receiver_count() > 0);
            if auto_stop && state.had_any && state.subs.is_empty() {
                // Checked and closed under the lock, so a concurrent
                // subscribe either lands before (and keeps us alive) or sees
                // `closed` and is refused.
                state.closed = true;
                cancel.cancel();
                return;
            }
            state
                .subs
                .iter()
                .map(|s| (s.tx.clone(), s.lossless))
                .collect()
        };
        for (tx, lossless) in targets {
            send_frame_backpressure(&tx, cancel, lossless, msg.clone());
        }
    }

    /// Refuse new subscriptions and drop the senders, so existing receivers
    /// see `Closed` once they have drained (after the final EOF).
    fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.subs.clear();
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SubscriberState> {
        // A panic while holding this lock cannot leave the state inconsistent
        // (push/retain/flag updates only), so recover from poisoning instead.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

enum DecoderType {
    Video(ffmpeg_next::codec::decoder::Video),
    Audio(ffmpeg_next::codec::decoder::Audio),
}

impl DecoderType {
    pub fn send_packet(
        &mut self,
        mut packet: RawPacket,
        decoder_time_base: Rational,
    ) -> anyhow::Result<()> {
        let time_base = packet.time_base();
        let packet = packet.get_mut();
        // Only rescale when time bases differ; rescale_ts can cause EINVAL for some codecs (e.g. WRAPPED_AVFRAME).
        if time_base != decoder_time_base {
            packet.rescale_ts(time_base, decoder_time_base);
        }
        match self {
            DecoderType::Video(video_decoder) => {
                video_decoder.send_packet(packet)?;
            }
            DecoderType::Audio(audio_decoder) => {
                audio_decoder.send_packet(packet)?;
            }
        }

        Ok(())
    }

    pub fn send_eof(&mut self) -> anyhow::Result<()> {
        match self {
            DecoderType::Video(video_decoder) => {
                video_decoder.send_eof()?;
            }
            DecoderType::Audio(audio_decoder) => {
                audio_decoder.send_eof()?;
            }
        }
        Ok(())
    }

    pub fn receive_frame(&mut self) -> anyhow::Result<Option<RawFrame>> {
        match self {
            DecoderType::Video(video_decoder) => {
                let mut frame = ffmpeg_next::frame::Video::empty();
                match video_decoder.receive_frame(&mut frame) {
                    Ok(()) => Ok(Some(RawFrame::Video(RawVideoFrame::from(frame)))),
                    Err(ffmpeg_next::Error::Eof) => Ok(None),
                    Err(ffmpeg_next::Error::Other { errno })
                        if errno == ffmpeg_next::util::error::EAGAIN =>
                    {
                        Ok(None)
                    }
                    Err(err) => Err(err.into()),
                }
            }
            DecoderType::Audio(audio_decoder) => {
                let mut frame = ffmpeg_next::frame::Audio::empty();
                match audio_decoder.receive_frame(&mut frame) {
                    Ok(()) => Ok(Some(RawFrame::Audio(RawAudioFrame::from(frame)))),
                    Err(ffmpeg_next::Error::Eof) => Ok(None),
                    Err(ffmpeg_next::Error::Other { errno })
                        if errno == ffmpeg_next::util::error::EAGAIN =>
                    {
                        Ok(None)
                    }
                    Err(err) => Err(err.into()),
                }
            }
        }
    }
}

pub struct Decoder {
    stream: AvStream,
    inner: DecoderType,
    decoder_time_base: Rational,
    /// True while decoding on a hardware codec; cleared after a runtime
    /// downgrade to software (see [`Decoder::send_packet`]).
    is_hw: bool,
    /// Name of the selected codec (to blacklist a failing hardware one).
    codec_name: String,
}

impl Decoder {
    fn open_video_decoder_with_codec(
        stream: &AvStream,
        codec: ffmpeg_next::Codec,
    ) -> anyhow::Result<(ffmpeg_next::codec::decoder::Video, Rational)> {
        let mut decoder_ctx = ffmpeg_next::codec::Context::new_with_codec(codec);
        unsafe {
            (*decoder_ctx.as_mut_ptr()).time_base = stream.time_base().into();
        }
        decoder_ctx.set_parameters(stream.parameters().clone())?;
        #[cfg(feature = "rockchip")]
        if codec.name().ends_with("_rkmpp") {
            // SAFETY: this context is exclusively owned until open. FFmpeg calls
            // the callback with a valid NONE-terminated pixel-format array.
            unsafe {
                (*decoder_ctx.as_mut_ptr()).get_format = Some(rockchip_software_format);
                if (*decoder_ctx.as_mut_ptr()).pix_fmt
                    == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
                {
                    (*decoder_ctx.as_mut_ptr()).pix_fmt =
                        ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_YUV420P;
                }
            }
        }
        let video_decoder = decoder_ctx.decoder().video()?;
        let decoder_time_base = video_decoder.time_base();
        Ok((video_decoder, decoder_time_base))
    }

    /// Open the default (software) decoder for this video stream, bypassing all
    /// hardware candidates. Used as the ultimate fallback in [`Decoder::new`]
    /// and for the runtime downgrade when a hardware decoder fails mid-stream.
    fn open_software_video(
        stream: &AvStream,
    ) -> anyhow::Result<(ffmpeg_next::codec::decoder::Video, Rational)> {
        let mut decoder_ctx = ffmpeg_next::codec::Context::new();
        unsafe {
            (*decoder_ctx.as_mut_ptr()).time_base = stream.time_base().into();
        }
        decoder_ctx.set_parameters(stream.parameters().clone())?;
        let video_decoder = decoder_ctx.decoder().video()?;
        let time_base = video_decoder.time_base();
        Ok((video_decoder, time_base))
    }

    pub fn new(stream: &AvStream) -> anyhow::Result<Self> {
        let s = if stream.is_video() {
            let mut selected_name = "default".to_string();
            let mut selected_is_hw = false;
            let mut first_hw_failure: Option<String> = None;
            let mut opened: Option<(ffmpeg_next::codec::decoder::Video, Rational)> = None;
            let candidates =
                hw::video_decoder_candidates(stream.parameters().id(), source_pixel_format(stream));
            for candidate in candidates {
                let Some(codec) = ffmpeg_next::decoder::find_by_name(&candidate.name) else {
                    continue;
                };
                match Self::open_video_decoder_with_codec(stream, codec) {
                    Ok(v) => {
                        selected_name = candidate.name.clone();
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
                            "video decoder candidate rejected: name={}, hw={}, reason={}",
                            candidate.name,
                            candidate.is_hw,
                            e
                        );
                    }
                }
            }
            if opened.is_none() {
                // ultimate software fallback: default codec from stream parameters
                opened = Some(Self::open_software_video(stream)?);
            }
            let (video_decoder, decoder_time_base) =
                opened.ok_or_else(|| anyhow::anyhow!("unable to open video decoder"))?;
            if selected_is_hw {
                log::info!(
                    "video decoder selected: {} (hardware), stream_index={}",
                    selected_name,
                    stream.index()
                );
            } else {
                if let Some(reason) = first_hw_failure {
                    log::info!(
                        "hardware decode unavailable, fallback to software: {}",
                        reason
                    );
                }
                log::info!(
                    "video decoder selected: {} (software), stream_index={}",
                    selected_name,
                    stream.index()
                );
            }

            Self {
                stream: stream.clone(),
                inner: DecoderType::Video(video_decoder),
                decoder_time_base,
                is_hw: selected_is_hw,
                codec_name: selected_name,
            }
        } else if stream.is_audio() {
            let mut decoder_ctx = ffmpeg_next::codec::Context::new();
            unsafe {
                (*decoder_ctx.as_mut_ptr()).time_base = stream.time_base().into();
            }
            decoder_ctx.set_parameters(stream.parameters().clone())?;
            let audio_decoder = decoder_ctx.decoder().audio()?;
            let decoder_time_base = audio_decoder.time_base();
            Self {
                stream: stream.clone(),
                inner: DecoderType::Audio(audio_decoder),
                decoder_time_base,
                is_hw: false,
                codec_name: "default".to_string(),
            }
        } else {
            return Err(anyhow::anyhow!("unsupported stream type"));
        };

        Ok(s)
    }

    pub fn send_packet(&mut self, packet: RawPacket) -> anyhow::Result<()> {
        // Keep a handle (Arc clone, no data copy) while on hardware so the
        // packet can be replayed into the software decoder after a downgrade.
        let retry = self.is_hw.then(|| packet.clone());
        match self.inner.send_packet(packet, self.decoder_time_base) {
            Ok(()) => Ok(()),
            // A hardware decoder can open cleanly yet fail on the first real
            // packet (e.g. QSV "MFX session" errors), with no built-in fallback.
            // Downgrade to software once and replay the failed packet: it is
            // often the stream's first keyframe, and dropping it leaves the
            // software decoder with nothing to decode until the next one.
            Err(e) if self.is_hw => {
                log::warn!(
                    "stream {}: hardware decode failed at runtime ({e:#}); \
                     falling back to software decoder",
                    self.stream.index()
                );
                hw::mark_runtime_failure(&self.codec_name);
                let (video_decoder, time_base) = Self::open_software_video(&self.stream)?;
                self.inner = DecoderType::Video(video_decoder);
                self.decoder_time_base = time_base;
                self.is_hw = false;
                match retry {
                    Some(packet) => self.inner.send_packet(packet, self.decoder_time_base),
                    None => Ok(()),
                }
            }
            Err(e) => Err(e),
        }
    }

    pub fn send_eof(&mut self) -> anyhow::Result<()> {
        self.inner.send_eof()
    }

    pub fn receive_frame(&mut self) -> anyhow::Result<Option<RawFrame>> {
        self.inner.receive_frame()
    }

    pub fn stream_index(&self) -> usize {
        self.stream.index()
    }
}

/// The source pixel format from the stream parameters, `Pixel::None` when
/// unknown. Read without opening a decoder.
fn source_pixel_format(stream: &AvStream) -> ffmpeg_next::format::Pixel {
    match ffmpeg_next::codec::Context::from_parameters(stream.parameters().clone()) {
        // SAFETY: the context is valid and owned here; pix_fmt is a plain field.
        Ok(ctx) => unsafe { (*ctx.as_ptr()).pix_fmt }.into(),
        Err(_) => ffmpeg_next::format::Pixel::None,
    }
}

/// Ask RKMPP to copy decoded pixels back to CPU memory for the bus's software
/// filters and subscribers. AFBC/DRM frames cannot be passed to swscale.
#[cfg(feature = "rockchip")]
unsafe extern "C" fn rockchip_software_format(
    _context: *mut ffmpeg_next::ffi::AVCodecContext,
    mut formats: *const ffmpeg_next::ffi::AVPixelFormat,
) -> ffmpeg_next::ffi::AVPixelFormat {
    use ffmpeg_next::ffi::{AV_PIX_FMT_FLAG_HWACCEL, AVPixelFormat, av_pix_fmt_desc_get};
    // SAFETY: FFmpeg guarantees a valid NONE-terminated array for get_format;
    // descriptors returned by av_pix_fmt_desc_get live for the entire process.
    unsafe {
        while *formats != AVPixelFormat::AV_PIX_FMT_NONE {
            let descriptor = av_pix_fmt_desc_get(*formats);
            if !descriptor.is_null() && (*descriptor).flags & (AV_PIX_FMT_FLAG_HWACCEL as u64) == 0
            {
                return *formats;
            }
            formats = formats.add(1);
        }
    }
    AVPixelFormat::AV_PIX_FMT_NONE
}

pub struct DecoderTask {
    cancel: CancellationToken,
    subscribers: FrameSubscribers,
    /// Stop once the last subscriber leaves (bus-managed decoders). Off for
    /// long-lived decoders whose subscribers come and go (e.g. audio mixer).
    auto_stop: bool,
}

impl DecoderTask {
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            subscribers: FrameSubscribers::default(),
            auto_stop: false,
        }
    }

    /// A decoder that stops itself when its last subscriber goes away, so a
    /// shared decoder nobody uses any more does not keep burning CPU.
    pub(crate) fn new_auto_stop() -> Self {
        let mut task = Self::new();
        task.auto_stop = true;
        task
    }

    /// Subscribe to decoded frames. `lossless` subscribers backpressure the
    /// shared decoder; lossy ones drop their own oldest frames when they lag.
    /// On a decoder that has ended, the receiver reports `Closed` at once.
    pub fn subscribe(&self, lossless: bool) -> RawFrameReceiver {
        self.try_subscribe(lossless).unwrap_or_else(closed_receiver)
    }

    /// Like [`Self::subscribe`], but `None` if the decoder has ended, so the
    /// caller can start a fresh one.
    pub(crate) fn try_subscribe(&self, lossless: bool) -> Option<RawFrameReceiver> {
        self.subscribers.add(lossless)
    }

    /// Still decoding and accepting subscribers.
    pub(crate) fn is_running(&self) -> bool {
        !self.cancel.is_cancelled() && !self.subscribers.is_closed()
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub async fn start(&self, decoder: Decoder, mut decoder_receiver: RawPacketReceiver) {
        log::info!(
            "decoder loop started, stream index: {}",
            decoder.stream_index()
        );
        let cancel_clone = self.cancel.clone();
        let subscribers = self.subscribers.clone();
        let auto_stop = self.auto_stop;
        /// Bounded queue: when decoder is slower than producer, back-pressure instead of unbounded growth (OOM).
        const PACKET_QUEUE_BOUND: usize = 16;
        tokio::spawn(async move {
            let (packet_tx, packet_rx) =
                std::sync::mpsc::sync_channel::<RawPacketCmd>(PACKET_QUEUE_BOUND);
            let current_stream_index = decoder.stream_index();

            let handle_cancel = cancel_clone.clone();
            let handle = tokio::task::spawn_blocking(move || {
                Self::decoder_loop(decoder, handle_cancel, packet_rx, subscribers, auto_stop)
            });
            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        break;
                    }
                    result = decoder_receiver.recv() => {
                        match result {
                            Ok(RawPacketCmd::Data(packet)) => {
                                if packet.index() != current_stream_index {
                                    continue;
                                }
                                // Async backpressure (never block this worker): if
                                // the decode loop is paused applying its own
                                // backpressure, yield so sibling tasks (e.g. the
                                // encoder relay) can run instead of deadlocking.
                                if Self::packet_send_backpressure(
                                    &packet_tx,
                                    &cancel_clone,
                                    RawPacketCmd::Data(packet),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                            Ok(RawPacketCmd::EOF) => {
                                let _ = Self::packet_send_backpressure(
                                    &packet_tx,
                                    &cancel_clone,
                                    RawPacketCmd::EOF,
                                )
                                .await;
                                break;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!(
                                    "decoder relay (stream {}): lagged, lost {} packets",
                                    current_stream_index,
                                    n
                                );
                            }
                            // Input gone (removed / bus dropped): dropping
                            // `packet_tx` below ends the decode loop.
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
            drop(packet_tx);
            let _ = handle.await;
        });
    }

    /// Send a packet into the bounded decode queue, waiting (async, so the
    /// worker stays free) for room instead of blocking the executor thread.
    /// Returns true if the decode loop's receiver has gone away.
    async fn packet_send_backpressure(
        tx: &std::sync::mpsc::SyncSender<RawPacketCmd>,
        cancel: &CancellationToken,
        msg: RawPacketCmd,
    ) -> bool {
        let mut pending = msg;
        loop {
            match tx.try_send(pending) {
                Ok(()) => return false,
                Err(std::sync::mpsc::TrySendError::Full(m)) => {
                    if cancel.is_cancelled() {
                        return false;
                    }
                    pending = m;
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return true,
            }
        }
    }

    fn decoder_loop(
        mut decoder: Decoder,
        cancel: CancellationToken,
        packet_rx: std::sync::mpsc::Receiver<RawPacketCmd>,
        subscribers: FrameSubscribers,
        auto_stop: bool,
    ) {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let mut eof = false;
            match packet_rx.recv_timeout(Duration::from_millis(1)) {
                Ok(packet) => {
                    match packet {
                        RawPacketCmd::Data(packet) => {
                            if let Err(e) = decoder.send_packet(packet) {
                                log::error!(
                                    "send packet error: {}\nbacktrace:\n{}",
                                    e,
                                    Backtrace::capture()
                                );
                                continue;
                            }
                        }
                        RawPacketCmd::EOF => {
                            if let Err(e) = decoder.send_eof() {
                                log::error!(
                                    "decoder send eof error: {}\nbacktrace:\n{}",
                                    e,
                                    Backtrace::capture()
                                );
                            }
                            eof = true;
                        }
                    };

                    'outer: loop {
                        match decoder.receive_frame() {
                            Ok(Some(frame)) => {
                                subscribers.publish(&cancel, auto_stop, RawFrameCmd::Data(frame));
                            }
                            Ok(None) => break 'outer,
                            Err(e) => {
                                log::error!(
                                    "receive frame error: {}\nbacktrace:\n{}",
                                    e,
                                    Backtrace::capture()
                                );
                                break 'outer;
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
                // Relay gone (input closed or task cancelled): nothing more
                // will arrive, so stop instead of spinning on a dead queue.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if eof {
                break;
            }
        }
        log::info!(
            "end of av decode task loop, stream base_time: {:#?}, decoder_time_base: {:#?}",
            decoder.stream.time_base(),
            decoder.decoder_time_base
        );
        // Backpressure EOF too, so it doesn't evict an unread tail frame.
        subscribers.publish(&cancel, false, RawFrameCmd::EOF);
        subscribers.close();
    }
}

impl Drop for DecoderTask {
    /// Dropping the task (bus teardown / input removal) stops the relay and the
    /// blocking decode loop, so they never outlive the bus.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
#[path = "decoder_test.rs"]
mod decoder_test;
