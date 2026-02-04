use std::collections::HashMap;

use ffmpeg_next::Dictionary;
use tokio_util::sync::CancellationToken;

use crate::{packet::RawPacket, stream::AvStream};

pub struct AvInputTask {
    cancel: CancellationToken,
    raw_chan: tokio::sync::broadcast::Sender<RawPacket>,
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
                        Ok(Some(packet)) => {
                            // Attempt to send, ignore send error (receiver dropped)
                            let _ = sender_clone.send(packet.into());
                        }
                        Ok(None) => {
                            // End of stream, break the loop
                            break;
                        }
                        Err(e) => {
                            log::error!("read packet error: {}", e);
                            break;
                        }
                    }
                }
            });

            tokio::select! {
                _ = handle => {
                    println!("read packet task finished");
                    cancel_clone.cancel();
                }
                _ = cancel_clone.cancelled() => {
                    println!("read packet task cancelled");
                }
            }
        });
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RawPacket> {
        self.raw_chan.subscribe()
    }

    pub fn get_cancel(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

pub struct AvInput {
    inner: ffmpeg_next::format::context::Input,
    streams: HashMap<usize, AvStream>,
}

impl AvInput {
    pub fn new(url: &str, options: Option<Dictionary>) -> anyhow::Result<Self> {
        let input = match options {
            Some(options) => ffmpeg_next::format::input_with_dictionary(url, options),
            None => ffmpeg_next::format::input(url),
        }?;

        let mut streams = HashMap::new();
        for stream in input.streams() {
            streams.insert(stream.index(), AvStream::from(stream));
        }

        Ok(Self {
            inner: input,
            streams,
        })
    }

    pub fn streams(&self) -> HashMap<usize, AvStream> {
        self.streams.clone()
    }

    pub fn read_packet(&mut self) -> anyhow::Result<Option<RawPacket>> {
        loop {
            match self.inner.packets().next() {
                Some((stream, packet)) => {
                    return Ok(Some((packet, stream.time_base()).into()));
                }
                None => {
                    return Err(anyhow::anyhow!("read eof"));
                }
            }
        }
    }
}
