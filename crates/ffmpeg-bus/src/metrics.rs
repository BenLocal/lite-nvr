//! Passive media telemetry. Counts media payload bytes, not transport overhead.
use crate::stream::AvStream;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct TrafficMetrics {
    pub bytes_total: u64,
    pub packets_total: u64,
    pub bytes_per_second: f64,
    pub packets_per_second: f64,
    pub last_packet_age_ms: Option<u64>,
}

struct Window {
    start: Instant,
    last: Option<Instant>,
    bytes: u64,
    packets: u64,
    total_bytes: u64,
    total_packets: u64,
    byte_rate: f64,
    packet_rate: f64,
}

pub(crate) struct Traffic(Mutex<Window>);
impl Default for Traffic {
    fn default() -> Self {
        Self(Mutex::new(Window {
            start: Instant::now(),
            last: None,
            bytes: 0,
            packets: 0,
            total_bytes: 0,
            total_packets: 0,
            byte_rate: 0.0,
            packet_rate: 0.0,
        }))
    }
}
impl Traffic {
    pub(crate) fn record(&self, bytes: usize) {
        self.record_at(bytes, Instant::now());
    }
    fn record_at(&self, bytes: usize, now: Instant) {
        let mut w = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // A new burst starts a fresh window; idle time must not dilute its rate.
        if w.last
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(2))
        {
            w.start = now;
            w.bytes = 0;
            w.packets = 0;
            w.byte_rate = 0.0;
            w.packet_rate = 0.0;
        }
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        w.bytes = w.bytes.saturating_add(bytes);
        w.packets = w.packets.saturating_add(1);
        w.total_bytes = w.total_bytes.saturating_add(bytes);
        w.total_packets = w.total_packets.saturating_add(1);
        w.last = Some(now);
        let seconds = now.duration_since(w.start).as_secs_f64();
        if seconds >= 1.0 {
            w.byte_rate = w.bytes as f64 / seconds;
            w.packet_rate = w.packets as f64 / seconds;
            w.start = now;
            w.bytes = 0;
            w.packets = 0;
        }
    }
    pub(crate) fn snapshot(&self) -> TrafficMetrics {
        self.snapshot_at(Instant::now())
    }
    fn snapshot_at(&self, now: Instant) -> TrafficMetrics {
        let w = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let age = w.last.map(|last| now.duration_since(last));
        let active = age.is_some_and(|age| age < Duration::from_secs(2));
        TrafficMetrics {
            bytes_total: w.total_bytes,
            packets_total: w.total_packets,
            bytes_per_second: if active { w.byte_rate } else { 0.0 },
            packets_per_second: if active { w.packet_rate } else { 0.0 },
            last_packet_age_ms: age.map(|age| u64::try_from(age.as_millis()).unwrap_or(u64::MAX)),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamMetrics {
    pub index: usize,
    pub kind: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub pixel_format: Option<String>,
    pub sample_rate: u32,
    pub channels: u32,
    pub traffic: TrafficMetrics,
}
impl StreamMetrics {
    pub(crate) fn from_stream(stream: &AvStream, traffic: TrafficMetrics) -> Self {
        let rate = stream.rate();
        let fps = (stream.is_video() && rate.0 > 0 && rate.1 > 0)
            .then(|| f64::from(rate.0) / f64::from(rate.1));
        // SAFETY: codec parameters are owned by the live stream and only read here.
        let format = unsafe { (*stream.parameters().as_ptr()).format };
        let pixel_format = if stream.is_video()
            && format >= 0
            && format < ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NB as i32
        {
            // SAFETY: AVPixelFormat values are contiguous in the validated 0..NB range.
            let name = unsafe {
                ffmpeg_next::ffi::av_get_pix_fmt_name(std::mem::transmute::<
                    i32,
                    ffmpeg_next::ffi::AVPixelFormat,
                >(format))
            };
            if name.is_null() {
                None
            } else {
                // SAFETY: a non-null FFmpeg format name is a static NUL-terminated string.
                Some(
                    unsafe { std::ffi::CStr::from_ptr(name) }
                        .to_string_lossy()
                        .into_owned(),
                )
            }
        } else {
            None
        };
        Self {
            index: stream.index(),
            kind: if stream.is_video() {
                "video"
            } else if stream.is_audio() {
                "audio"
            } else {
                "other"
            }
            .into(),
            codec: stream.parameters().id().name().into(),
            width: stream.width(),
            height: stream.height(),
            fps,
            pixel_format,
            sample_rate: stream.sample_rate(),
            channels: stream.channels(),
            traffic,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CodecMetrics {
    pub stream_index: usize,
    pub kind: String,
    pub codec: String,
    pub hardware: bool,
    pub running: bool,
    pub traffic: TrafficMetrics,
}
#[derive(Clone, Default)]
pub(crate) struct CodecProbe(Arc<Mutex<CodecMetrics>>, Arc<Traffic>);
impl CodecProbe {
    pub(crate) fn update(&self, stream: &AvStream, codec: &str, hardware: bool) {
        let mut info = self.0.lock().unwrap_or_else(|e| e.into_inner());
        info.stream_index = stream.index();
        let kind = if stream.is_video() { "video" } else { "audio" };
        if info.kind != kind {
            info.kind = kind.into();
        }
        if info.codec != codec {
            info.codec = codec.into();
        }
        info.hardware = hardware;
    }
    pub(crate) fn record(&self, bytes: usize) {
        self.1.record(bytes);
    }
    pub(crate) fn snapshot(&self, running: bool) -> CodecMetrics {
        let mut info = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        info.running = running;
        info.traffic = self.1.snapshot();
        info
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct OutputMetrics {
    pub id: String,
    pub destination: String,
    pub traffic_available: bool,
    pub stream: StreamMetrics,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct BusMetrics {
    pub running: bool,
    pub input: Vec<StreamMetrics>,
    pub decoders: Vec<CodecMetrics>,
    pub encoders: Vec<CodecMetrics>,
    pub output: Vec<OutputMetrics>,
}

#[cfg(test)]
#[path = "metrics_test.rs"]
mod metrics_test;
