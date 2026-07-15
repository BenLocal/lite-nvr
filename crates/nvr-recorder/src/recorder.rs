use std::time::Duration;

use chrono::{DateTime, Utc};
use ffmpeg_bus::input::{AvInput, AvInputTask};
use ffmpeg_bus::packet::{RawPacket, RawPacketCmd};
use ffmpeg_bus::stream::AvStream;
use ffmpeg_next::Dictionary;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::{RecorderConfig, RtspTransport, TrackSelect};
use crate::info::SegmentInfo;
use crate::rotation::{is_split_point, should_rotate};
use crate::segment::{SegmentWriter, tb_to_us};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Video,
    Audio,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Selected {
    pub video: Option<usize>,
    pub audio: Option<usize>,
}

/// Resolve the input stream indices to record, honoring the track selection.
/// Errors only when a specifically requested single kind is absent (or when
/// `Both` finds neither video nor audio).
pub(crate) fn select_streams(
    streams: impl IntoIterator<Item = (usize, MediaKind)>,
    tracks: TrackSelect,
) -> anyhow::Result<Selected> {
    let mut video = None;
    let mut audio = None;
    for (idx, kind) in streams {
        match kind {
            MediaKind::Video if video.is_none() => video = Some(idx),
            MediaKind::Audio if audio.is_none() => audio = Some(idx),
            _ => {}
        }
    }
    let want_v = matches!(tracks, TrackSelect::Video | TrackSelect::Both);
    let want_a = matches!(tracks, TrackSelect::Audio | TrackSelect::Both);
    let sel = Selected {
        video: if want_v { video } else { None },
        audio: if want_a { audio } else { None },
    };
    match tracks {
        TrackSelect::Video if sel.video.is_none() => {
            anyhow::bail!("no video stream in source")
        }
        TrackSelect::Audio if sel.audio.is_none() => {
            anyhow::bail!("no audio stream in source")
        }
        TrackSelect::Both if sel.video.is_none() && sel.audio.is_none() => {
            anyhow::bail!("source has neither video nor audio")
        }
        _ => {}
    }
    Ok(sel)
}

/// Exponential backoff: attempt 0 -> base, doubling, capped at max.
pub(crate) fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let factor = 1u128.checked_shl(attempt).unwrap_or(u128::MAX);
    let ms = base.as_millis().saturating_mul(factor).min(max.as_millis());
    Duration::from_millis(ms as u64)
}

/// The timestamp origin for a packet: its DTS (fallback PTS) in microseconds.
fn pkt_origin_us(pkt: &RawPacket) -> i64 {
    let tb = pkt.time_base();
    let ts = pkt.dts().or_else(|| pkt.pts()).unwrap_or(0);
    tb_to_us(ts, tb.numerator(), tb.denominator())
}

/// Build a segment filename from the strftime `pattern`, the segment start
/// wall-clock `dt`, and the container `ext` (e.g. "rec_20231114_221320.ts").
pub(crate) fn segment_filename(pattern: &str, ext: &str, dt: DateTime<Utc>) -> String {
    format!("{}.{}", dt.format(pattern), ext)
}

pub struct Recorder {
    config: RecorderConfig,
    tx: mpsc::Sender<SegmentInfo>,
}

impl Recorder {
    /// Build a recorder plus the channel on which completed segments arrive.
    pub fn new(config: RecorderConfig) -> (Recorder, mpsc::Receiver<SegmentInfo>) {
        let (tx, rx) = mpsc::channel(16);
        (Recorder { config, tx }, rx)
    }

    /// Record until `cancel` fires or the stream ends and reconnect is exhausted.
    pub async fn run(self, cancel: CancellationToken) -> anyhow::Result<()> {
        let mut attempt: u32 = 0;
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            match self.record_once(&cancel).await {
                Ok(()) => return Ok(()), // cancelled cleanly inside the session
                Err(e) => {
                    log::warn!("nvr-recorder session ended: {e:#}");
                    match self.config.reconnect.max_retries {
                        Some(0) => return Ok(()),
                        Some(n) if attempt >= n => return Ok(()),
                        _ => {}
                    }
                    let delay = backoff_delay(
                        attempt,
                        self.config.reconnect.base_delay,
                        self.config.reconnect.max_delay,
                    );
                    attempt = attempt.saturating_add(1);
                    tokio::select! {
                        _ = cancel.cancelled() => return Ok(()),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
            }
        }
    }

    async fn record_once(&self, cancel: &CancellationToken) -> anyhow::Result<()> {
        // 1. Open the RTSP input off the async runtime (blocking connect).
        let url = self.config.url.clone();
        let transport = self.config.transport;
        let timeout_us = self.config.open_timeout.as_micros().to_string();
        let input = tokio::task::spawn_blocking(move || {
            let mut opts = Dictionary::new();
            opts.set(
                "rtsp_transport",
                match transport {
                    RtspTransport::Tcp => "tcp",
                    RtspTransport::Udp => "udp",
                },
            );
            opts.set("timeout", &timeout_us);
            AvInput::new(&url, None, Some(opts))
        })
        .await??;

        // 2. Resolve the streams to record.
        let kinds: Vec<(usize, MediaKind)> = input
            .streams()
            .iter()
            .map(|(i, s)| {
                let k = if s.is_video() {
                    MediaKind::Video
                } else if s.is_audio() {
                    MediaKind::Audio
                } else {
                    MediaKind::Other
                };
                (*i, k)
            })
            .collect();
        let sel = select_streams(kinds, self.config.tracks)?;
        let has_video = sel.video.is_some();
        let video_index = sel.video;
        let mut selected: Vec<AvStream> = Vec::new();
        if let Some(vi) = sel.video {
            selected.push(input.streams().get(&vi).unwrap().clone());
        }
        if let Some(ai) = sel.audio {
            selected.push(input.streams().get(&ai).unwrap().clone());
        }

        // 3. Start the demux reader.
        let task = AvInputTask::new();
        let mut rx = task.subscribe();
        task.start(input).await;

        std::fs::create_dir_all(&self.config.output_dir)?;
        let mut writer: Option<SegmentWriter> = None;

        // 4. Packet loop.
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    self.close_writer(&mut writer).await?;
                    task.stop();
                    return Ok(());
                }
                cmd = rx.recv() => {
                    let cmd = match cmd {
                        Ok(c) => c,
                        Err(_) => {
                            self.close_writer(&mut writer).await?;
                            task.stop();
                            anyhow::bail!("input channel closed");
                        }
                    };
                    match cmd {
                        RawPacketCmd::EOF => {
                            self.close_writer(&mut writer).await?;
                            task.stop();
                            anyhow::bail!("end of stream");
                        }
                        RawPacketCmd::Data(pkt) => {
                            let idx = pkt.index();
                            let is_selected =
                                sel.video == Some(idx) || sel.audio == Some(idx);
                            if !is_selected {
                                continue;
                            }
                            let pkt_is_video = video_index == Some(idx);
                            let split_ok = is_split_point(has_video, pkt_is_video, pkt.is_key());
                            let now = Utc::now();

                            match writer.as_ref() {
                                None => {
                                    // Wait for the first legal split point to start file #1.
                                    if !split_ok {
                                        continue;
                                    }
                                    let base_us = pkt_origin_us(&pkt);
                                    writer = Some(self.open_segment(&selected, base_us, now)?);
                                }
                                Some(w) => {
                                    let cur_us = pkt_origin_us(&pkt);
                                    let elapsed = std::time::Duration::from_micros(
                                        (cur_us - w.base_us()).max(0) as u64,
                                    );
                                    if split_ok
                                        && should_rotate(
                                            self.config.align_to_wall_clock,
                                            self.config.segment_time,
                                            w.start_wall(),
                                            now,
                                            elapsed,
                                        )
                                    {
                                        let finished = writer.take().unwrap().finish()?;
                                        let _ = self.tx.send(finished).await;
                                        let base_us = pkt_origin_us(&pkt);
                                        writer =
                                            Some(self.open_segment(&selected, base_us, now)?);
                                    }
                                }
                            }

                            if let Some(w) = writer.as_mut() {
                                w.write(pkt)?;
                            }
                        }
                    }
                }
            }
        }
    }

    fn open_segment(
        &self,
        streams: &[AvStream],
        base_us: i64,
        now: DateTime<Utc>,
    ) -> anyhow::Result<SegmentWriter> {
        let fname = segment_filename(
            &self.config.filename_pattern,
            self.config.container.extension(),
            now,
        );
        let path = self.config.output_dir.join(fname);
        SegmentWriter::open(path, self.config.container, streams, base_us, now)
    }

    async fn close_writer(&self, writer: &mut Option<SegmentWriter>) -> anyhow::Result<()> {
        if let Some(w) = writer.take() {
            let info = w.finish()?;
            let _ = self.tx.send(info).await;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "recorder_test.rs"]
mod recorder_test;
