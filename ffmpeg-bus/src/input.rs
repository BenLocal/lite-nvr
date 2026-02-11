use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;

use ffmpeg_next::Dictionary;
use tokio_util::sync::CancellationToken;

use crate::{
    packet::{RawPacket, RawPacketCmd, RawPacketReceiver, RawPacketSender},
    stream::AvStream,
};

pub struct AvInputTask {
    cancel: CancellationToken,
    raw_chan: RawPacketSender,
}

impl AvInputTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        let (sender, _) = tokio::sync::broadcast::channel(1024);

        Self {
            cancel,
            raw_chan: sender,
        }
    }

    pub async fn start(&self, mut input: AvInput) {
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();
        tokio::spawn(async move {
            let cancel_inner = cancel_clone.clone();
            let handle = tokio::task::spawn_blocking(move || {
                loop {
                    if cancel_inner.is_cancelled() {
                        break;
                    }
                    match input.read_packet() {
                        Some(packet) => {
                            // Attempt to send, ignore send error (receiver dropped)
                            let _ = sender_clone.send(RawPacketCmd::Data(packet));
                        }
                        None => {
                            // End of stream, break the loop
                            println!("end of read input stream:");
                            for (index, stream) in input.streams.iter() {
                                println!(
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
                    println!("read input packet task finished");
                    cancel_clone.cancel();
                }
                _ = cancel_clone.cancelled() => {
                    println!("read input packet task cancelled");
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
                let ctx = ffmpeg_next::format::open_with(
                    path,
                    &Format::Input(fmt),
                    Dictionary::new(),
                )?;
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
        loop {
            match self.inner.packets().next() {
                Some((stream, packet)) => {
                    return Some((packet, stream.time_base()).into());
                }
                None => {
                    // End of stream, break the loop
                    return None;
                }
            }
        }
    }
}
