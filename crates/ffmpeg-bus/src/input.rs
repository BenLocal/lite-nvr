use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;
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
                    match input.read_packet() {
                        Some(packet) => {
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
                        None => {
                            // End of stream, break the loop
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

pub struct AvInput {
    inner: ffmpeg_next::format::context::Input,
    streams: HashMap<usize, AvStream>,
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
        use ffmpeg_next::format::format::Format;

        let path = Path::new(url);
        let input = match (format, options) {
            (Some(fmt_name), Some(opts)) => {
                let fmt = Self::find_input_format(fmt_name)?;
                let ctx = ffmpeg_next::format::open_with(path, &Format::Input(fmt), opts)?;
                ctx.input()
            }
            (Some(fmt_name), None) => {
                let fmt = Self::find_input_format(fmt_name)?;
                let ctx =
                    ffmpeg_next::format::open_with(path, &Format::Input(fmt), Dictionary::new())?;
                ctx.input()
            }
            (None, Some(opts)) => ffmpeg_next::format::input_with_dictionary(path, opts)?,
            (None, None) => ffmpeg_next::format::input(path)?,
        };

        let mut streams = HashMap::new();
        for stream in input.streams() {
            streams.insert(stream.index(), AvStream::from(stream));
        }

        Ok(Self {
            inner: input,
            streams,
        })
    }

    pub fn streams(&self) -> &HashMap<usize, AvStream> {
        &self.streams
    }

    pub fn read_packet(&mut self) -> Option<RawPacket> {
        // One packet per call, or None at end of stream. No loop here: both match
        // arms returned, so a `loop` never actually iterated (clippy::never_loop).
        self.inner
            .packets()
            .next()
            .map(|(stream, packet)| (packet, stream.time_base()).into())
    }
}

#[cfg(test)]
#[path = "input_test.rs"]
mod input_test;
