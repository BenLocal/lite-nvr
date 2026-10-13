//! One encoded audio program shared by independent output consumers.

use crate::packet::{RawPacket, RawPacketCmd, RawPacketReceiver, RawPacketSender};
use crate::stream::AvStream;
use ffmpeg_next::{Rational, Rescale};
use std::sync::Arc;

#[derive(Clone)]
pub struct SharedEncodedAudio {
    stream: Arc<AvStream>,
    packets: RawPacketSender,
    revision: u64,
}

impl SharedEncodedAudio {
    pub fn new(stream: AvStream, packets: RawPacketSender) -> anyhow::Result<Self> {
        anyhow::ensure!(
            stream.is_audio(),
            "shared program must describe an audio stream"
        );
        anyhow::ensure!(
            stream.sample_rate() > 0 && stream.channels() > 0,
            "invalid program audio format"
        );
        Ok(Self {
            stream: Arc::new(stream),
            packets,
            revision: 0,
        })
    }

    pub fn stream(&self) -> &AvStream {
        &self.stream
    }

    pub fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Each subscriber has its own bounded broadcast position. A slow output
    /// loses its own oldest packets without blocking the program encoder.
    pub fn subscribe(&self) -> RawPacketReceiver {
        self.packets.subscribe()
    }

    /// Align this live program to an input session and end with that input,
    /// rather than keeping a finite video/file output alive forever.
    pub(crate) fn aligned_packets(
        &self,
        reference: AvStream,
        input: RawPacketReceiver,
        index: usize,
        cancel: tokio_util::sync::CancellationToken,
    ) -> impl futures::Stream<Item = Result<RawPacketCmd, u64>> + Send + Sync + 'static {
        let time_base = self.stream.time_base();
        let audio = self.subscribe();
        futures::stream::unfold(
            (
                input,
                audio,
                None::<AudioTimeline>,
                false,
                reference,
                cancel,
            ),
            move |(mut input, mut audio, mut timeline, ended, reference, cancel)| async move {
                if ended {
                    return None;
                }
                loop {
                    let result = tokio::select! {
                        _ = cancel.cancelled() => Some(Ok(RawPacketCmd::EOF)),
                        r = input.recv() => match r {
                            Ok(RawPacketCmd::Data(p)) if p.index() == reference.index() => {
                                if timeline.is_none() {
                                    if let Some(pts) = p.pts().or(p.dts()) {
                                        timeline = Some(AudioTimeline::new(time_base, pts.rescale(p.time_base(), time_base)));
                                    }
                                }
                                None
                            }
                            Ok(RawPacketCmd::EOF) | Err(tokio::sync::broadcast::error::RecvError::Closed) => Some(Ok(RawPacketCmd::EOF)),
                            _ => None,
                        },
                        r = audio.recv() => match r {
                            Ok(RawPacketCmd::Data(p)) => match timeline.as_mut() {
                                Some(clock) => match clock.map(p, index) {
                                    Ok(p) => Some(Ok(RawPacketCmd::Data(p))),
                                    Err(e) => { log::warn!("program timestamp mapping: {e:#}"); None }
                                },
                                None => None,
                            },
                            Ok(RawPacketCmd::EOF) | Err(tokio::sync::broadcast::error::RecvError::Closed) => Some(Ok(RawPacketCmd::EOF)),
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!("program output lagged, skipped {n} packets"); Some(Err(n))
                            }
                        },
                    };
                    if let Some(packet) = result {
                        let ended = matches!(packet, Ok(RawPacketCmd::EOF));
                        return Some((packet, (input, audio, timeline, ended, reference, cancel)));
                    }
                }
            },
        )
    }
}

/// Packet-header mapping for one output. Codec payload remains shared.
pub(crate) struct AudioTimeline {
    time_base: Rational,
    anchor: i64,
    origin: Option<i64>,
}

impl AudioTimeline {
    pub(crate) fn new(time_base: Rational, anchor: i64) -> Self {
        Self {
            time_base,
            anchor,
            origin: None,
        }
    }

    pub(crate) fn map(&mut self, packet: RawPacket, index: usize) -> anyhow::Result<RawPacket> {
        let source_time_base = packet.time_base();
        let pts = packet
            .pts()
            .or(packet.dts())
            .ok_or_else(|| anyhow::anyhow!("program packet has no timestamp"))?
            .rescale(source_time_base, self.time_base);
        let origin = *self.origin.get_or_insert(pts);
        let offset = self
            .anchor
            .checked_sub(origin)
            .ok_or_else(|| anyhow::anyhow!("audio timestamp offset overflow"))?;
        use ffmpeg_next::codec::packet::{Mut, Ref};
        let mut mapped = ffmpeg_next::Packet::empty();
        // SAFETY: destination is an initialized empty AVPacket, source stays
        // alive through this call. Only the independent packet header is
        // subsequently changed; the shared codec payload is never written.
        let result = unsafe {
            ffmpeg_next::ffi::av_packet_ref(mapped.as_mut_ptr(), packet.packet().as_ptr())
        };
        if result < 0 {
            return Err(ffmpeg_next::Error::from(result).into());
        }
        mapped.rescale_ts(source_time_base, self.time_base);
        for value in [mapped.pts(), mapped.dts()].into_iter().flatten() {
            anyhow::ensure!(
                value.checked_add(offset).is_some(),
                "audio timestamp overflow"
            );
        }
        mapped.set_pts(mapped.pts().map(|v| v + offset));
        mapped.set_dts(mapped.dts().map(|v| v + offset));
        mapped.set_stream(index);
        Ok((mapped, self.time_base).into())
    }
}

#[cfg(test)]
#[path = "shared_audio_test.rs"]
mod shared_audio_test;
