//! One output file over an `AvOutput`, plus the pure timestamp math used to
//! reset each segment to a ~0 origin (emulating ffmpeg `-reset_timestamps 1`).

use std::path::PathBuf;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ffmpeg_bus::output::AvOutput;
use ffmpeg_bus::packet::RawPacket;
use ffmpeg_bus::stream::AvStream;

use crate::config::Container;
use crate::info::{AudioMeta, SegmentInfo, VideoMeta, codec_name};

pub(crate) struct SegmentWriter {
    output: AvOutput,
    path: PathBuf,
    start_wall: DateTime<Utc>,
    /// Common origin (microseconds) subtracted from every packet's PTS/DTS.
    base_us: i64,
    /// Stream whose PTS drives the measured duration (video if present, else audio).
    primary_index: usize,
    first_primary_us: Option<i64>,
    last_primary_us: i64,
    size_bytes: u64,
    video: Option<VideoMeta>,
    audio: Option<AudioMeta>,
}

impl SegmentWriter {
    /// Open a new output file and register the selected streams (stream-copy).
    /// `base_us` is the common timestamp origin for this segment; `start_wall`
    /// is its wall-clock start (also the source of the filename).
    ///
    /// Note: although this is a stream-copy, `AvOutput::add_stream` requires an
    /// encoder to be registered for each stream's codec id in the current FFmpeg
    /// build; a codec with no encoder fails here (before any packet is written).
    pub(crate) fn open(
        path: PathBuf,
        container: Container,
        streams: &[AvStream],
        base_us: i64,
        start_wall: DateTime<Utc>,
    ) -> anyhow::Result<Self> {
        let path_str = path.to_string_lossy().to_string();
        let mut output = AvOutput::new(&path_str, Some(container.muxer_name()), None)?;
        let mut video = None;
        let mut audio = None;
        let mut primary_index = streams.first().map(|s| s.index()).unwrap_or(0);
        for s in streams {
            output.add_stream(s)?;
            if s.is_video() {
                primary_index = s.index();
                video = Some(VideoMeta {
                    codec: codec_name(s.parameters().id()),
                    width: s.width(),
                    height: s.height(),
                    fps: s.fps(),
                });
            } else if s.is_audio() {
                audio = Some(AudioMeta {
                    codec: codec_name(s.parameters().id()),
                    sample_rate: s.sample_rate(),
                    channels: s.channels(),
                });
            }
        }
        Ok(Self {
            output,
            path,
            start_wall,
            base_us,
            primary_index,
            first_primary_us: None,
            last_primary_us: 0,
            size_bytes: 0,
            video,
            audio,
        })
    }

    pub(crate) fn base_us(&self) -> i64 {
        self.base_us
    }

    pub(crate) fn start_wall(&self) -> DateTime<Utc> {
        self.start_wall
    }

    /// Offset the packet to this segment's origin, then mux it (stream-copy).
    pub(crate) fn write(&mut self, mut pkt: RawPacket) -> anyhow::Result<()> {
        let tb = pkt.time_base();
        let (num, den) = (tb.numerator(), tb.denominator());
        let off = us_to_tb(self.base_us, num, den);
        {
            let p = pkt.get_mut();
            if let Some(pts) = p.pts() {
                p.set_pts(Some((pts - off).max(0)));
            }
            if let Some(dts) = p.dts() {
                p.set_dts(Some((dts - off).max(0)));
            }
        }
        let idx = pkt.index();
        self.size_bytes += pkt.size() as u64;
        if idx == self.primary_index
            && let Some(pts) = pkt.pts()
        {
            let us = tb_to_us(pts, num, den);
            if self.first_primary_us.is_none() {
                self.first_primary_us = Some(us);
            }
            self.last_primary_us = us;
        }
        self.output.write_packet(idx, pkt)?;
        Ok(())
    }

    /// Write the trailer and return the finished segment's metadata.
    pub(crate) fn finish(mut self) -> anyhow::Result<SegmentInfo> {
        self.output.finish()?;
        let first = self.first_primary_us.unwrap_or(0);
        let duration = duration_seconds(first, self.last_primary_us);
        let end_wall = self.start_wall + ChronoDuration::milliseconds((duration * 1000.0) as i64);
        Ok(SegmentInfo {
            path: self.path,
            start_wall: self.start_wall,
            end_wall,
            duration,
            size_bytes: self.size_bytes,
            video: self.video,
            audio: self.audio,
        })
    }
}

/// Rescale a timestamp in `tb` (num/den seconds per tick) to microseconds.
pub(crate) fn tb_to_us(ts: i64, tb_num: i32, tb_den: i32) -> i64 {
    if tb_den == 0 {
        return 0;
    }
    (ts as i128 * tb_num as i128 * 1_000_000 / tb_den as i128) as i64
}

/// Rescale a microsecond value into ticks of `tb` (num/den seconds per tick).
pub(crate) fn us_to_tb(us: i64, tb_num: i32, tb_den: i32) -> i64 {
    if tb_num == 0 {
        return 0;
    }
    (us as i128 * tb_den as i128 / (tb_num as i128 * 1_000_000)) as i64
}

/// Segment duration in seconds from first/last primary-stream PTS (microseconds).
pub(crate) fn duration_seconds(first_us: i64, last_us: i64) -> f64 {
    (last_us - first_us).max(0) as f64 / 1_000_000.0
}

#[cfg(test)]
#[path = "segment_test.rs"]
mod segment_test;
