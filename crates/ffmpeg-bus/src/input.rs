use std::collections::HashMap;
use std::ffi::CString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ffmpeg_next::Dictionary;
use tokio_util::sync::CancellationToken;

use crate::{
    packet::{RawPacket, RawPacketCmd, RawPacketReceiver, RawPacketSender},
    stream::AvStream,
};

pub struct AvInputTask {
    cancel: CancellationToken,
    raw_chan: RawPacketSender,
    /// Set once a lossless (File/Net) output exists: the reader then waits for
    /// its slowest subscriber instead of overwriting unread packets.
    lossless: Arc<AtomicBool>,
}

impl AvInputTask {
    /// Input packet channel. Bounded per-frame size; balance memory vs avoiding Lagged drop.
    const PACKET_CHAN_CAP: usize = 4096;
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        let (sender, _) = tokio::sync::broadcast::channel(Self::PACKET_CHAN_CAP);

        Self {
            cancel,
            raw_chan: sender,
            lossless: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Switch the reader to lossless mode (one-way). A fast source (e.g. a file
    /// read in a burst) then cannot outrun subscribers and make them lag.
    pub fn set_lossless(&self) {
        self.lossless.store(true, Ordering::Relaxed);
    }

    pub async fn start(&self, mut input: AvInput) {
        let interrupter = input.interrupter();
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();
        let lossless = self.lossless.clone();
        tokio::spawn(async move {
            let cancel_inner = cancel_clone.clone();
            let handle = tokio::task::spawn_blocking(move || {
                loop {
                    if cancel_inner.is_cancelled() {
                        break;
                    }
                    match input.read() {
                        ReadOutcome::Packet(packet) => {
                            if lossless.load(Ordering::Relaxed) {
                                while sender_clone.len() >= Self::PACKET_CHAN_CAP
                                    && sender_clone.receiver_count() > 0
                                    && !cancel_inner.is_cancelled()
                                {
                                    std::thread::sleep(Duration::from_millis(2));
                                }
                            }
                            // Attempt to send, ignore send error (receiver dropped)
                            let _ = sender_clone.send(RawPacketCmd::Data(packet));
                        }
                        // Nothing ready yet (some devices): retry, staying
                        // responsive to cancellation.
                        ReadOutcome::Again => std::thread::sleep(Duration::from_millis(2)),
                        outcome @ (ReadOutcome::End | ReadOutcome::Failed(_)) => {
                            // A read error (camera gone, network timeout or
                            // reset) ends the stream like EOF does, so
                            // downstream finalizes and the session can be
                            // restarted instead of hanging on a dead input.
                            if let ReadOutcome::Failed(e) = outcome {
                                log::warn!("input read failed ({e}); ending the stream");
                            }
                            log::info!("end of read input stream:");
                            for (index, stream) in input.streams.iter() {
                                log::info!(
                                    "stream index: {}, stream id: {:#?}, time_base: {:#?}",
                                    index,
                                    stream.parameters().id(),
                                    stream.time_base()
                                );
                            }
                            let _ = sender_clone.send(RawPacketCmd::EOF);
                            break;
                        }
                    }
                }

                drop(sender_clone);
            });

            tokio::select! {
                _ = handle => {
                    log::info!("read input packet task finished");
                    cancel_clone.cancel();
                }
                _ = cancel_clone.cancelled() => {
                    // Abort a read blocked on a silent source, or the reader
                    // (and process shutdown) would wait on it indefinitely.
                    interrupter.interrupt();
                    log::info!("read input packet task cancelled");
                }
            }
        });
    }

    pub fn subscribe(&self) -> RawPacketReceiver {
        self.raw_chan.subscribe()
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// Whether the reader waits for its slowest subscriber (see `set_lossless`).
    pub(crate) fn is_lossless(&self) -> bool {
        self.lossless.load(Ordering::Relaxed)
    }

    /// Still reading (or not yet started). The reader cancels itself at EOF.
    pub(crate) fn is_running(&self) -> bool {
        !self.cancel.is_cancelled()
    }
}

impl Drop for AvInputTask {
    /// Dropping the task stops the blocking reader so it never outlives the bus.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Result of one [`AvInput::read`].
pub enum ReadOutcome {
    Packet(RawPacket),
    /// Nothing available yet (EAGAIN): try again.
    Again,
    /// End of stream.
    End,
    /// The read failed (I/O error, timeout, connection reset, ...).
    Failed(ffmpeg_next::Error),
}

/// Flags FFmpeg's interrupt callback polls during blocking I/O (open, read).
/// Set, they make the blocked call return `AVERROR_EXIT` right away.
#[derive(Default)]
struct InterruptState {
    /// This input was stopped (see [`Interrupter`]).
    local: AtomicBool,
    /// Owner-wide stop (e.g. the whole bus shutting down mid-open).
    external: Option<Arc<AtomicBool>>,
}

impl InterruptState {
    fn interrupted(&self) -> bool {
        self.local.load(Ordering::Relaxed)
            || self
                .external
                .as_ref()
                .is_some_and(|f| f.load(Ordering::Relaxed))
    }
}

extern "C" fn interrupt_callback(opaque: *mut std::ffi::c_void) -> std::ffi::c_int {
    // SAFETY: `opaque` is the `InterruptState` owned by the `AvInput` whose
    // format context calls us; `AvInput` drops that context before the state
    // (field order), so the pointer is valid for every call.
    let state = unsafe { &*(opaque as *const InterruptState) };
    std::ffi::c_int::from(state.interrupted())
}

/// Handle that aborts an [`AvInput`]'s blocking I/O from another thread.
#[derive(Clone)]
pub struct Interrupter(Arc<InterruptState>);

impl Interrupter {
    pub fn interrupt(&self) {
        self.0.local.store(true, Ordering::Relaxed);
    }
}

pub struct AvInput {
    // Field order matters: `inner` (the format context, which may call the
    // interrupt callback) must drop before `interrupt`.
    inner: ffmpeg_next::format::context::Input,
    streams: HashMap<usize, AvStream>,
    interrupt: Arc<InterruptState>,
}

impl AvInput {
    /// Resolve input format by name (e.g. "x11grab", "v4l2") via FFmpeg's av_find_input_format.
    fn find_input_format(name: &str) -> anyhow::Result<ffmpeg_next::format::format::Input> {
        let cname = CString::new(name)
            .map_err(|e| anyhow::anyhow!("invalid format name {:?}: {}", name, e))?;
        let ptr = unsafe { ffmpeg_next::ffi::av_find_input_format(cname.as_ptr()) };
        if ptr.is_null() {
            return Err(anyhow::anyhow!("input format not found: {}", name));
        }
        Ok(unsafe { ffmpeg_next::format::format::Input::wrap(ptr as *mut _) })
    }

    pub fn new(
        url: &str,
        format: Option<&str>,
        options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        Self::open(url, format, options, None)
    }

    /// Open `url` with an interrupt callback, so the (possibly long) open and
    /// every later read can be aborted: by [`AvInput::interrupter`], or when
    /// `external` is set (an owner-wide stop flag).
    pub fn open(
        url: &str,
        format: Option<&str>,
        options: Option<Dictionary>,
        external: Option<Arc<AtomicBool>>,
    ) -> anyhow::Result<Self> {
        let interrupt = Arc::new(InterruptState {
            local: AtomicBool::new(false),
            external,
        });
        let fmt = format.map(Self::find_input_format).transpose()?;
        let url_c =
            CString::new(url).map_err(|e| anyhow::anyhow!("invalid url {:?}: {}", url, e))?;
        // SAFETY: standard avformat_open_input sequence. The context is
        // allocated here so the interrupt callback is in place before any
        // I/O; on open failure FFmpeg frees it, on probe failure we close it.
        // The callback's opaque points into `interrupt`, which outlives the
        // context (see the field order of `AvInput`).
        let input = unsafe {
            let mut ps = ffmpeg_next::ffi::avformat_alloc_context();
            if ps.is_null() {
                anyhow::bail!("avformat_alloc_context failed");
            }
            (*ps).interrupt_callback = ffmpeg_next::ffi::AVIOInterruptCB {
                callback: Some(interrupt_callback),
                opaque: Arc::as_ptr(&interrupt) as *mut std::ffi::c_void,
            };
            let mut opts = options.map_or(std::ptr::null_mut(), |o| o.disown());
            let res = ffmpeg_next::ffi::avformat_open_input(
                &mut ps,
                url_c.as_ptr(),
                fmt.as_ref().map_or(std::ptr::null(), |f| f.as_ptr()),
                &mut opts,
            );
            // Leftover (unused) options are ours to free.
            drop(Dictionary::own(opts));
            if res < 0 {
                return Err(ffmpeg_next::Error::from(res).into());
            }
            let res = ffmpeg_next::ffi::avformat_find_stream_info(ps, std::ptr::null_mut());
            if res < 0 {
                ffmpeg_next::ffi::avformat_close_input(&mut ps);
                return Err(ffmpeg_next::Error::from(res).into());
            }
            ffmpeg_next::format::context::Input::wrap(ps)
        };

        let mut streams = HashMap::new();
        for stream in input.streams() {
            streams.insert(stream.index(), AvStream::from(stream));
        }

        Ok(Self {
            inner: input,
            streams,
            interrupt,
        })
    }

    /// A handle to abort this input's blocking I/O (e.g. a read stuck on a
    /// silent network source) from another thread.
    pub fn interrupter(&self) -> Interrupter {
        Interrupter(self.interrupt.clone())
    }

    pub fn streams(&self) -> &HashMap<usize, AvStream> {
        &self.streams
    }

    /// Read one packet. Unlike ffmpeg-next's `packets()` iterator, which
    /// retries every non-EOF error forever inside `next()` (so a dead RTSP
    /// camera hangs the reader, unable to even see cancellation), errors are
    /// reported to the caller.
    pub fn read(&mut self) -> ReadOutcome {
        let mut packet = ffmpeg_next::Packet::empty();
        match packet.read(&mut self.inner) {
            Ok(()) => {
                let time_base = self
                    .inner
                    .stream(packet.stream())
                    .map(|s| s.time_base())
                    // A packet's stream always exists; 0/1 is a never-hit fallback.
                    .unwrap_or(ffmpeg_next::Rational(0, 1));
                ReadOutcome::Packet((packet, time_base).into())
            }
            Err(ffmpeg_next::Error::Eof) => ReadOutcome::End,
            Err(ffmpeg_next::Error::Other { errno })
                if errno == ffmpeg_next::util::error::EAGAIN =>
            {
                ReadOutcome::Again
            }
            Err(e) => ReadOutcome::Failed(e),
        }
    }

    /// One packet, or `None` at end of stream or on a read error (retrying
    /// only "try again"). Convenience for synchronous callers.
    pub fn read_packet(&mut self) -> Option<RawPacket> {
        loop {
            match self.read() {
                ReadOutcome::Packet(packet) => return Some(packet),
                ReadOutcome::Again => std::thread::sleep(Duration::from_millis(1)),
                ReadOutcome::End | ReadOutcome::Failed(_) => return None,
            }
        }
    }
}

#[cfg(test)]
#[path = "input_test.rs"]
mod input_test;
