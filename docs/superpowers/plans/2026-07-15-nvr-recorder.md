# RTSP Segment Recorder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `crates/nvr-recorder` library (plus a runnable example) that records one RTSP source into time-sliced files by stream-copy, emitting per-segment metadata.

**Architecture:** In-process orchestration over existing `ffmpeg-bus` primitives — `AvInput`/`AvInputTask` demux RTSP into a packet broadcast, a `SegmentWriter` wraps `AvOutput` for one file, and a `Recorder` loop rotates the file on a time boundary at a video keyframe (or any packet when audio-only), resetting per-segment timestamps and reconnecting with backoff. Completed segments are delivered on an `mpsc<SegmentInfo>`.

**Tech Stack:** Rust (edition 2024), `ffmpeg-bus` (path dep) + `ffmpeg-next`, `tokio`/`tokio-util`, `chrono`, `serde`/`serde_json`, `clap`+`env_logger` (example only).

## Global Constraints

- Rust **edition 2024**; `snake_case`; run `cargo fmt` before every commit.
- Do **not** modify `crates/ffmpeg-bus` or the `nvr` crate. This crate is standalone.
- **Stream-copy only** — never decode/encode. A codec a container can't hold is a config error, not an auto-transcode.
- Tests colocated as `<module>_test.rs`, imported at the end of each source file via `#[cfg(test)] #[path = "<module>_test.rs"] mod <module>_test;` (repo convention in `CLAUDE.md`).
- The crate links libav, so **all** `cargo test`/`build`/`check` commands run from the repo root prefixed with `LD_LIBRARY_PATH=$PWD/ffmpeg/lib` (e.g. `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder`). `FFMPEG_DIR` defaults to `./ffmpeg`.
- Default container **TS** (`mpegts`); MP4 (`mp4`) and MKV (`matroska`) selectable.
- Reconnect is **on by default** (`max_retries: None` = forever until cancelled).

---

### Task 1: Scaffold crate + config

**Files:**
- Create: `crates/nvr-recorder/Cargo.toml`
- Create: `crates/nvr-recorder/src/lib.rs`
- Create: `crates/nvr-recorder/src/config.rs`
- Create: `crates/nvr-recorder/src/config_test.rs`
- Modify: `Cargo.toml` (workspace `members`)

**Interfaces:**
- Produces: `RtspTransport{Tcp,Udp}`, `TrackSelect{Video,Audio,Both}`, `Container{Ts,Mp4,Mkv}` with `Container::extension()->&'static str` and `Container::muxer_name()->&'static str`; `ReconnectPolicy{max_retries:Option<u32>, base_delay:Duration, max_delay:Duration}` (+ `Default`); `RecorderConfig{url,transport,tracks,segment_time,align_to_wall_clock,container,output_dir,filename_pattern,open_timeout,reconnect}` with `RecorderConfig::new(url: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self`.

- [ ] **Step 1: Create the crate manifest**

Create `crates/nvr-recorder/Cargo.toml`:

```toml
[package]
name = "nvr-recorder"
version = "0.1.0"
edition = "2024"
publish = false
description = "Record one RTSP source into time-sliced stream-copy segments with per-segment metadata."

[dependencies]
ffmpeg-bus = { path = "../ffmpeg-bus" }
ffmpeg-next = { workspace = true }
tokio = { workspace = true }
tokio-util = { workspace = true }
anyhow = { workspace = true }
chrono = { workspace = true }
log = { workspace = true }
serde = { workspace = true }

# serde_json/clap/env_logger are only used by info_test and the `record`
# example; examples can use dev-dependencies, so they live here.
[dev-dependencies]
serde_json = { workspace = true }
env_logger = { workspace = true }
clap = { version = "4", features = ["derive"] }
```

The `examples/record.rs` binary is auto-discovered by cargo — no `[[example]]` block is needed.

- [ ] **Step 2: Register the crate in the workspace**

In root `Cargo.toml`, add the member to the `members` list (after `"crates/nvr-yt-dlp",`):

```toml
    "crates/nvr-yt-dlp",
    "crates/nvr-recorder",
```

- [ ] **Step 3: Write `config.rs`**

Create `crates/nvr-recorder/src/config.rs`:

```rust
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtspTransport {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSelect {
    Video,
    Audio,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Ts,
    Mp4,
    Mkv,
}

impl Container {
    /// File extension for this container.
    pub fn extension(self) -> &'static str {
        match self {
            Container::Ts => "ts",
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
        }
    }

    /// FFmpeg muxer (format) name for this container.
    pub fn muxer_name(self) -> &'static str {
        match self {
            Container::Ts => "mpegts",
            Container::Mp4 => "mp4",
            Container::Mkv => "matroska",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// `None` = reconnect forever (until cancelled); `Some(0)` = never reconnect.
    pub max_retries: Option<u32>,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_retries: None,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(16),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub url: String,
    pub transport: RtspTransport,
    pub tracks: TrackSelect,
    pub segment_time: Duration,
    pub align_to_wall_clock: bool,
    pub container: Container,
    pub output_dir: PathBuf,
    /// strftime pattern for the segment start wall-clock (no extension).
    pub filename_pattern: String,
    pub open_timeout: Duration,
    pub reconnect: ReconnectPolicy,
}

impl RecorderConfig {
    /// Build a config with documented defaults; override fields as needed.
    pub fn new(url: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            transport: RtspTransport::Tcp,
            tracks: TrackSelect::Both,
            segment_time: Duration::from_secs(60),
            align_to_wall_clock: false,
            container: Container::Ts,
            output_dir: output_dir.into(),
            filename_pattern: "rec_%Y%m%d_%H%M%S".to_string(),
            open_timeout: Duration::from_secs(5),
            reconnect: ReconnectPolicy::default(),
        }
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
```

- [ ] **Step 4: Write `lib.rs`**

Create `crates/nvr-recorder/src/lib.rs` (modules for later tasks referenced now so the crate has a stable shape; they are created in their own tasks):

```rust
//! Record one RTSP source into time-sliced stream-copy segments.

pub mod config;

pub use config::{Container, ReconnectPolicy, RecorderConfig, RtspTransport, TrackSelect};
```

- [ ] **Step 5: Write the failing test**

Create `crates/nvr-recorder/src/config_test.rs`:

```rust
use super::*;

#[test]
fn defaults_are_documented() {
    let c = RecorderConfig::new("rtsp://x", "/tmp/out");
    assert_eq!(c.url, "rtsp://x");
    assert_eq!(c.transport, RtspTransport::Tcp);
    assert_eq!(c.tracks, TrackSelect::Both);
    assert_eq!(c.segment_time, Duration::from_secs(60));
    assert!(!c.align_to_wall_clock);
    assert_eq!(c.container, Container::Ts);
    assert_eq!(c.filename_pattern, "rec_%Y%m%d_%H%M%S");
    assert_eq!(c.open_timeout, Duration::from_secs(5));
    assert_eq!(c.reconnect.max_retries, None);
    assert_eq!(c.reconnect.base_delay, Duration::from_secs(1));
    assert_eq!(c.reconnect.max_delay, Duration::from_secs(16));
}

#[test]
fn container_maps_to_muxer_and_extension() {
    assert_eq!(Container::Ts.muxer_name(), "mpegts");
    assert_eq!(Container::Ts.extension(), "ts");
    assert_eq!(Container::Mp4.muxer_name(), "mp4");
    assert_eq!(Container::Mkv.muxer_name(), "matroska");
}
```

- [ ] **Step 6: Run test to verify it fails, then passes**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder`
Expected: the crate compiles and both tests PASS. (If you wrote the test before `config.rs`, it fails to compile first — that's the red step.)

- [ ] **Step 7: Format & commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/Cargo.toml crates/nvr-recorder/src Cargo.toml
git commit -m "feat(nvr-recorder): scaffold crate and config"
```

---

### Task 2: Segment metadata (`info.rs`)

**Files:**
- Create: `crates/nvr-recorder/src/info.rs`
- Create: `crates/nvr-recorder/src/info_test.rs`
- Modify: `crates/nvr-recorder/src/lib.rs`

**Interfaces:**
- Produces: `VideoMeta{codec:String,width:u32,height:u32,fps:f32}`, `AudioMeta{codec:String,sample_rate:u32,channels:u32}`, `SegmentInfo{path:PathBuf,start_wall:DateTime<Utc>,end_wall:DateTime<Utc>,duration:f64,size_bytes:u64,video:Option<VideoMeta>,audio:Option<AudioMeta>}` (all `Serialize`); `pub(crate) fn codec_name(id: ffmpeg_next::codec::Id) -> String`.

- [ ] **Step 1: Write `info.rs`**

Create `crates/nvr-recorder/src/info.rs`:

```rust
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VideoMeta {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioMeta {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentInfo {
    pub path: PathBuf,
    pub start_wall: DateTime<Utc>,
    pub end_wall: DateTime<Utc>,
    pub duration: f64,
    pub size_bytes: u64,
    pub video: Option<VideoMeta>,
    pub audio: Option<AudioMeta>,
}

/// FFmpeg's canonical short name for a codec id (e.g. "h264", "aac").
pub(crate) fn codec_name(id: ffmpeg_next::codec::Id) -> String {
    unsafe {
        let ptr = ffmpeg_next::ffi::avcodec_get_name(id.into());
        if ptr.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
#[path = "info_test.rs"]
mod info_test;
```

- [ ] **Step 2: Export from `lib.rs`**

In `crates/nvr-recorder/src/lib.rs`, add below the `config` module lines:

```rust
pub mod info;

pub use info::{AudioMeta, SegmentInfo, VideoMeta};
```

- [ ] **Step 3: Write the failing test**

Create `crates/nvr-recorder/src/info_test.rs`:

```rust
use chrono::DateTime;

use super::*;

#[test]
fn codec_name_resolves_known_ids() {
    assert_eq!(codec_name(ffmpeg_next::codec::Id::H264), "h264");
    assert_eq!(codec_name(ffmpeg_next::codec::Id::AAC), "aac");
}

#[test]
fn segment_info_serializes_expected_shape() {
    let s = SegmentInfo {
        path: "/tmp/rec.ts".into(),
        start_wall: DateTime::from_timestamp(1000, 0).unwrap(),
        end_wall: DateTime::from_timestamp(1010, 0).unwrap(),
        duration: 10.0,
        size_bytes: 123,
        video: Some(VideoMeta {
            codec: "h264".into(),
            width: 640,
            height: 360,
            fps: 25.0,
        }),
        audio: None,
    };
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j["duration"], 10.0);
    assert_eq!(j["size_bytes"], 123);
    assert_eq!(j["video"]["codec"], "h264");
    assert_eq!(j["video"]["width"], 640);
    assert!(j["audio"].is_null());
}
```

- [ ] **Step 4: Run tests**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder`
Expected: PASS (4 tests total now).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/src
git commit -m "feat(nvr-recorder): SegmentInfo metadata and codec_name helper"
```

---

### Task 3: Rotation logic (`rotation.rs`)

**Files:**
- Create: `crates/nvr-recorder/src/rotation.rs`
- Create: `crates/nvr-recorder/src/rotation_test.rs`
- Modify: `crates/nvr-recorder/src/lib.rs`

**Interfaces:**
- Produces: `pub fn next_boundary(start: DateTime<Utc>, period: Duration) -> DateTime<Utc>`; `pub fn is_split_point(has_video: bool, pkt_is_video: bool, pkt_is_key: bool) -> bool`; `pub fn should_rotate(align_to_wall_clock: bool, segment_time: Duration, segment_start_wall: DateTime<Utc>, now: DateTime<Utc>, elapsed_media: Duration) -> bool`.

- [ ] **Step 1: Write `rotation.rs`**

Create `crates/nvr-recorder/src/rotation.rs`:

```rust
use std::time::Duration;

use chrono::{DateTime, Utc};

/// Next epoch-aligned multiple of `period` strictly after `start`.
/// Used for wall-clock-aligned segmentation (e.g. rotate on each minute).
pub fn next_boundary(start: DateTime<Utc>, period: Duration) -> DateTime<Utc> {
    let period_ms = (period.as_millis() as i64).max(1);
    let start_ms = start.timestamp_millis();
    let next_ms = (start_ms.div_euclid(period_ms) + 1) * period_ms;
    DateTime::from_timestamp_millis(next_ms).expect("boundary in range")
}

/// Whether the current packet is a legal point to close a segment.
/// With video present, only a video keyframe may split (a stream-copy that
/// begins mid-GOP is unplayable); audio-only may split on any packet.
pub fn is_split_point(has_video: bool, pkt_is_video: bool, pkt_is_key: bool) -> bool {
    if has_video {
        pkt_is_video && pkt_is_key
    } else {
        true
    }
}

/// Given we are already at a legal split point, decide whether to rotate now.
pub fn should_rotate(
    align_to_wall_clock: bool,
    segment_time: Duration,
    segment_start_wall: DateTime<Utc>,
    now: DateTime<Utc>,
    elapsed_media: Duration,
) -> bool {
    if align_to_wall_clock {
        now >= next_boundary(segment_start_wall, segment_time)
    } else {
        elapsed_media >= segment_time
    }
}

#[cfg(test)]
#[path = "rotation_test.rs"]
mod rotation_test;
```

- [ ] **Step 2: Export from `lib.rs`**

Add to `crates/nvr-recorder/src/lib.rs`:

```rust
pub mod rotation;
```

- [ ] **Step 3: Write the failing tests**

Create `crates/nvr-recorder/src/rotation_test.rs`:

```rust
use super::*;

fn t(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap()
}

#[test]
fn boundary_rounds_up_to_next_multiple() {
    // period 60s, start at 00:00:12.000 -> next boundary 00:01:00.000
    assert_eq!(next_boundary(t(12_000), Duration::from_secs(60)), t(60_000));
}

#[test]
fn boundary_is_strictly_after_when_on_multiple() {
    assert_eq!(next_boundary(t(60_000), Duration::from_secs(60)), t(120_000));
}

#[test]
fn split_point_with_video_requires_video_keyframe() {
    assert!(is_split_point(true, true, true));
    assert!(!is_split_point(true, true, false));
    assert!(!is_split_point(true, false, true));
}

#[test]
fn split_point_audio_only_is_anywhere() {
    assert!(is_split_point(false, false, false));
}

#[test]
fn rotate_on_media_duration() {
    let time = Duration::from_secs(10);
    assert!(!should_rotate(false, time, t(0), t(0), Duration::from_secs(9)));
    assert!(should_rotate(false, time, t(0), t(0), Duration::from_secs(10)));
}

#[test]
fn rotate_on_wall_clock_boundary() {
    let time = Duration::from_secs(60);
    let start = t(12_000); // boundary at 60_000
    assert!(!should_rotate(true, time, start, t(59_000), Duration::from_secs(47)));
    assert!(should_rotate(true, time, start, t(60_000), Duration::from_secs(48)));
}
```

- [ ] **Step 4: Run tests**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder rotation`
Expected: PASS (6 rotation tests).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/src
git commit -m "feat(nvr-recorder): pure rotation and wall-clock boundary logic"
```

---

### Task 4: Timestamp/duration math (`segment.rs` pure functions)

**Files:**
- Create: `crates/nvr-recorder/src/segment.rs`
- Create: `crates/nvr-recorder/src/segment_test.rs`
- Modify: `crates/nvr-recorder/src/lib.rs`

**Interfaces:**
- Produces: `pub(crate) fn tb_to_us(ts: i64, tb_num: i32, tb_den: i32) -> i64`; `pub(crate) fn us_to_tb(us: i64, tb_num: i32, tb_den: i32) -> i64`; `pub(crate) fn duration_seconds(first_us: i64, last_us: i64) -> f64`.

- [ ] **Step 1: Write `segment.rs` (pure functions only for now)**

Create `crates/nvr-recorder/src/segment.rs`:

```rust
//! One output file over an `AvOutput`, plus the pure timestamp math used to
//! reset each segment to a ~0 origin (emulating ffmpeg `-reset_timestamps 1`).

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
```

- [ ] **Step 2: Add the module to `lib.rs`**

Add to `crates/nvr-recorder/src/lib.rs`:

```rust
mod segment;
```

- [ ] **Step 3: Write the failing tests**

Create `crates/nvr-recorder/src/segment_test.rs`:

```rust
use super::*;

#[test]
fn tb_to_us_90khz() {
    // 90000 ticks at tb 1/90000 == 1s == 1_000_000 us
    assert_eq!(tb_to_us(90_000, 1, 90_000), 1_000_000);
}

#[test]
fn us_to_tb_90khz() {
    assert_eq!(us_to_tb(1_000_000, 1, 90_000), 90_000);
}

#[test]
fn round_trip_us_tb() {
    let tb = (1, 48_000);
    let us = tb_to_us(24_000, tb.0, tb.1); // 0.5s
    assert_eq!(us, 500_000);
    assert_eq!(us_to_tb(us, tb.0, tb.1), 24_000);
}

#[test]
fn duration_seconds_basic_and_clamped() {
    assert_eq!(duration_seconds(0, 10_000_000), 10.0);
    assert_eq!(duration_seconds(500, 100), 0.0);
}
```

- [ ] **Step 4: Run tests**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder segment`
Expected: PASS (4 segment tests).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/src
git commit -m "feat(nvr-recorder): per-segment timestamp and duration math"
```

---

### Task 5: Engine helpers — stream selection & backoff (`recorder.rs` pure functions)

**Files:**
- Create: `crates/nvr-recorder/src/recorder.rs`
- Create: `crates/nvr-recorder/src/recorder_test.rs`
- Modify: `crates/nvr-recorder/src/lib.rs`

**Interfaces:**
- Produces: `pub(crate) enum MediaKind{Video,Audio,Other}`; `pub(crate) struct Selected{video:Option<usize>,audio:Option<usize>}`; `pub(crate) fn select_streams(streams: impl IntoIterator<Item=(usize,MediaKind)>, tracks: TrackSelect) -> anyhow::Result<Selected>`; `pub(crate) fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration`.

- [ ] **Step 1: Write `recorder.rs` (helpers only for now)**

Create `crates/nvr-recorder/src/recorder.rs`:

```rust
use std::time::Duration;

use crate::config::TrackSelect;

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

#[cfg(test)]
#[path = "recorder_test.rs"]
mod recorder_test;
```

- [ ] **Step 2: Add the module to `lib.rs`**

Add to `crates/nvr-recorder/src/lib.rs`:

```rust
pub mod recorder;
```

- [ ] **Step 3: Write the failing tests**

Create `crates/nvr-recorder/src/recorder_test.rs`:

```rust
use super::*;
use crate::config::TrackSelect;

#[test]
fn selects_both_when_present() {
    let s = select_streams(
        [(0, MediaKind::Video), (1, MediaKind::Audio)],
        TrackSelect::Both,
    )
    .unwrap();
    assert_eq!(
        s,
        Selected {
            video: Some(0),
            audio: Some(1)
        }
    );
}

#[test]
fn audio_track_ignores_video_stream() {
    let s = select_streams(
        [(0, MediaKind::Video), (1, MediaKind::Audio)],
        TrackSelect::Audio,
    )
    .unwrap();
    assert_eq!(
        s,
        Selected {
            video: None,
            audio: Some(1)
        }
    );
}

#[test]
fn errors_when_requested_video_absent() {
    assert!(select_streams([(0, MediaKind::Audio)], TrackSelect::Video).is_err());
}

#[test]
fn both_errors_only_when_nothing_present() {
    assert!(select_streams([(0, MediaKind::Other)], TrackSelect::Both).is_err());
    // video-only source with Both is fine (audio None)
    let s = select_streams([(0, MediaKind::Video)], TrackSelect::Both).unwrap();
    assert_eq!(s.video, Some(0));
    assert_eq!(s.audio, None);
}

#[test]
fn backoff_doubles_and_caps() {
    let base = Duration::from_secs(1);
    let max = Duration::from_secs(16);
    assert_eq!(backoff_delay(0, base, max), Duration::from_secs(1));
    assert_eq!(backoff_delay(1, base, max), Duration::from_secs(2));
    assert_eq!(backoff_delay(4, base, max), Duration::from_secs(16));
    assert_eq!(backoff_delay(100, base, max), Duration::from_secs(16));
}
```

- [ ] **Step 4: Run tests**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder recorder`
Expected: PASS (5 recorder tests).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/src
git commit -m "feat(nvr-recorder): stream selection and reconnect backoff"
```

---

### Task 6: SegmentWriter (`segment.rs` — the `AvOutput` wrapper)

**Files:**
- Modify: `crates/nvr-recorder/src/segment.rs`

**Interfaces:**
- Consumes: `tb_to_us`, `us_to_tb`, `duration_seconds` (Task 4); `SegmentInfo`, `VideoMeta`, `AudioMeta`, `codec_name` (Task 2); `Container` (Task 1); `ffmpeg_bus::output::AvOutput`, `ffmpeg_bus::packet::RawPacket`, `ffmpeg_bus::stream::AvStream`.
- Produces: `pub(crate) struct SegmentWriter` with `SegmentWriter::open(path: PathBuf, container: Container, streams: &[AvStream], base_us: i64, start_wall: DateTime<Utc>) -> anyhow::Result<Self>`, `fn write(&mut self, pkt: RawPacket) -> anyhow::Result<()>`, `fn finish(self) -> anyhow::Result<SegmentInfo>`, `fn base_us(&self) -> i64`, `fn start_wall(&self) -> DateTime<Utc>`.

> Note: `SegmentWriter`'s write path drives libav muxing and is validated by the smoke test (Task 9); this task's automated gate is that the crate compiles and all prior unit tests still pass.

- [ ] **Step 1: Add imports and the struct to `segment.rs`**

At the top of `crates/nvr-recorder/src/segment.rs`, above the pure functions, add:

```rust
use std::path::{Path, PathBuf};

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

    pub(crate) fn path(&self) -> &Path {
        &self.path
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
```

> If `let ... && let ...` chaining is rejected by the toolchain, split it into a nested `if let Some(pts) = pkt.pts() { if idx == self.primary_index { … } }`.

- [ ] **Step 2: Verify the crate compiles and prior tests pass**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder`
Expected: compiles; all 19 unit tests from Tasks 1–5 PASS. `SegmentWriter` has no new unit test (its mux path needs a real stream — validated in Task 9).

- [ ] **Step 3: Format & commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/src
git commit -m "feat(nvr-recorder): SegmentWriter wrapping AvOutput with timestamp reset"
```

---

### Task 7: Recorder engine loop (`recorder.rs`)

**Files:**
- Modify: `crates/nvr-recorder/src/recorder.rs`
- Modify: `crates/nvr-recorder/src/lib.rs`

**Interfaces:**
- Consumes: `RecorderConfig`, `RtspTransport`, `TrackSelect` (Task 1); `SegmentInfo` (Task 2); `is_split_point`, `should_rotate` (Task 3); `SegmentWriter`, `tb_to_us` (Tasks 4/6); `select_streams`, `backoff_delay`, `MediaKind` (Task 5); `ffmpeg_bus::input::{AvInput,AvInputTask}`, `ffmpeg_bus::packet::{RawPacket,RawPacketCmd}`, `ffmpeg_bus::stream::AvStream`.
- Produces: `pub struct Recorder`; `Recorder::new(config: RecorderConfig) -> (Recorder, tokio::sync::mpsc::Receiver<SegmentInfo>)`; `async fn run(self, cancel: CancellationToken) -> anyhow::Result<()>`.

> Runtime behavior is validated by the smoke test (Task 9); this task's automated gate is that the crate compiles and all prior unit tests still pass.

- [ ] **Step 1: Add engine imports at the top of `recorder.rs`**

Add these imports above the existing `use` lines in `crates/nvr-recorder/src/recorder.rs`:

```rust
use chrono::{DateTime, Utc};
use ffmpeg_bus::input::{AvInput, AvInputTask};
use ffmpeg_bus::packet::{RawPacket, RawPacketCmd};
use ffmpeg_bus::stream::AvStream;
use ffmpeg_next::Dictionary;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::{RecorderConfig, RtspTransport};
use crate::info::SegmentInfo;
use crate::rotation::{is_split_point, should_rotate};
use crate::segment::{SegmentWriter, tb_to_us};
```

- [ ] **Step 2: Append the `Recorder` implementation to `recorder.rs`**

Add below the helper functions (above the `#[cfg(test)]` line) in `crates/nvr-recorder/src/recorder.rs`:

```rust
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
        task.start(input);

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
```

- [ ] **Step 3: Export `Recorder` from `lib.rs`**

Add to `crates/nvr-recorder/src/lib.rs`:

```rust
pub use recorder::Recorder;
```

- [ ] **Step 4: Add the filename-formatting test**

Append to `crates/nvr-recorder/src/recorder_test.rs`:

```rust
#[test]
fn segment_filename_uses_strftime() {
    // epoch 1_700_000_000 == 2023-11-14T22:13:20Z
    let dt = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let name = segment_filename("rec_%Y%m%d_%H%M%S", "ts", dt);
    assert_eq!(name, "rec_20231114_221320.ts");
}
```

- [ ] **Step 5: Verify the crate compiles and tests pass**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder`
Expected: compiles; all 20 unit tests PASS (the new filename test plus the 19 from Tasks 1–5). Loop runtime is exercised in Task 9.

- [ ] **Step 6: Format & commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/src
git commit -m "feat(nvr-recorder): recorder engine loop with rotation and reconnect"
```

---

### Task 8: Example CLI (`examples/record.rs`)

**Files:**
- Create: `crates/nvr-recorder/examples/record.rs`

**Interfaces:**
- Consumes: `RecorderConfig`, `Recorder`, `SegmentInfo`, `Container`, `TrackSelect`, `RtspTransport` from the crate; `clap`, `env_logger`, `tokio`, `serde_json`.

- [ ] **Step 1: Write the example**

Create `crates/nvr-recorder/examples/record.rs`:

```rust
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use nvr_recorder::{Container, Recorder, RecorderConfig, RtspTransport, TrackSelect};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(about = "Record an RTSP source into time-sliced segment files.")]
struct Args {
    /// RTSP URL, e.g. rtsp://127.0.0.1:8554/stream
    #[arg(long)]
    url: String,
    /// Output directory (created if missing).
    #[arg(long, default_value = "./records")]
    dir: PathBuf,
    /// Segment length in seconds.
    #[arg(long, default_value_t = 60)]
    segment_time: u64,
    /// Tracks to record: video | audio | both.
    #[arg(long, default_value = "both")]
    tracks: String,
    /// Container: ts | mp4 | mkv.
    #[arg(long, default_value = "ts")]
    container: String,
    /// Align segment boundaries to the wall clock (e.g. each minute).
    #[arg(long, default_value_t = false)]
    align: bool,
}

fn parse_tracks(s: &str) -> TrackSelect {
    match s {
        "video" => TrackSelect::Video,
        "audio" => TrackSelect::Audio,
        _ => TrackSelect::Both,
    }
}

fn parse_container(s: &str) -> Container {
    match s {
        "mp4" => Container::Mp4,
        "mkv" => Container::Mkv,
        _ => Container::Ts,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    ffmpeg_bus::init()?;

    let args = Args::parse();
    let mut config = RecorderConfig::new(args.url, &args.dir);
    config.transport = RtspTransport::Tcp;
    config.tracks = parse_tracks(&args.tracks);
    config.container = parse_container(&args.container);
    config.segment_time = Duration::from_secs(args.segment_time);
    config.align_to_wall_clock = args.align;

    std::fs::create_dir_all(&args.dir)?;
    let manifest_path = args.dir.join("manifest.jsonl");
    let mut manifest = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&manifest_path)?;

    let (recorder, mut rx) = Recorder::new(config);
    let cancel = CancellationToken::new();

    let run_cancel = cancel.clone();
    let handle = tokio::spawn(async move { recorder.run(run_cancel).await });

    // Ctrl-C -> graceful stop.
    let sig_cancel = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("interrupt received, stopping recorder");
        sig_cancel.cancel();
    });

    while let Some(info) = rx.recv().await {
        let line = serde_json::to_string(&info)?;
        writeln!(manifest, "{line}")?;
        manifest.flush()?;
        log::info!(
            "segment: {} ({:.3}s, {} bytes)",
            info.path.display(),
            info.duration,
            info.size_bytes
        );
    }

    handle.await??;
    log::info!("recorder stopped; manifest at {}", manifest_path.display());
    Ok(())
}
```

- [ ] **Step 2: Verify the example builds**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo build -p nvr-recorder --example record`
Expected: builds with no errors.

- [ ] **Step 3: Commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/examples
git commit -m "feat(nvr-recorder): example CLI writing a jsonl manifest"
```

---

### Task 9: Smoke test against `dummy-rtsp-camera`

**Files:**
- Create: `crates/nvr-recorder/tests/smoke.rs`

**Interfaces:**
- Consumes: the whole public crate API + the repo's `examples/dummy-rtsp-camera` RTSP fixture.

> This test is `#[ignore]` because it needs a live RTSP source; it reads `RTSP_TEST_URL` and skips its assertions if unset. It is the runtime validation for Tasks 6–8.

- [ ] **Step 1: Write the ignored integration test**

Create `crates/nvr-recorder/tests/smoke.rs`:

```rust
use std::time::Duration;

use nvr_recorder::{Container, Recorder, RecorderConfig, TrackSelect};
use tokio_util::sync::CancellationToken;

/// Record a real RTSP source for ~12s at 4s segments and assert we produced
/// at least two segments with monotonic start times. Requires `RTSP_TEST_URL`.
///
/// Run with, e.g.:
///   RTSP_TEST_URL=rtsp://127.0.0.1:8554/stream \
///   LD_LIBRARY_PATH=$PWD/ffmpeg/lib \
///   cargo test -p nvr-recorder --test smoke -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn records_segments_from_live_rtsp() {
    let Ok(url) = std::env::var("RTSP_TEST_URL") else {
        eprintln!("RTSP_TEST_URL not set; skipping");
        return;
    };
    ffmpeg_bus::init().unwrap();

    let dir = std::env::temp_dir().join("nvr-recorder-smoke");
    let _ = std::fs::remove_dir_all(&dir);

    let mut config = RecorderConfig::new(url, &dir);
    config.tracks = TrackSelect::Both;
    config.container = Container::Ts;
    config.segment_time = Duration::from_secs(4);

    let (recorder, mut rx) = Recorder::new(config);
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let handle = tokio::spawn(async move { recorder.run(run_cancel).await });

    let mut segments = Vec::new();
    let deadline = tokio::time::sleep(Duration::from_secs(12));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            Some(info) = rx.recv() => segments.push(info),
        }
    }
    cancel.cancel();
    // Drain any final segment emitted during shutdown.
    while let Ok(Some(info)) =
        tokio::time::timeout(Duration::from_secs(3), rx.recv()).await
    {
        segments.push(info);
    }
    let _ = handle.await;

    assert!(
        segments.len() >= 2,
        "expected >= 2 segments, got {}",
        segments.len()
    );
    for w in segments.windows(2) {
        assert!(
            w[1].start_wall >= w[0].start_wall,
            "segment start times must be monotonic"
        );
    }
    for s in &segments {
        let meta = std::fs::metadata(&s.path).expect("segment file exists");
        assert!(meta.len() > 0, "segment file must be non-empty");
    }
}
```

- [ ] **Step 2: Confirm it compiles and is skipped by default**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr-recorder`
Expected: builds; the smoke test is listed as `ignored`, all unit tests PASS.

- [ ] **Step 3: Run the real smoke test manually**

In one terminal, start the RTSP fixture (thin launcher around oddity-rtsp-server):

```bash
LD_LIBRARY_PATH=$PWD/ffmpeg/lib:$PWD/target/debug/deps \
  cargo run -p dummy-rtsp-camera
# note the RTSP URL it prints, e.g. rtsp://127.0.0.1:8554/<path>
```

In another terminal, run the ignored test against it:

```bash
RTSP_TEST_URL=rtsp://127.0.0.1:8554/<path> \
LD_LIBRARY_PATH=$PWD/ffmpeg/lib \
cargo test -p nvr-recorder --test smoke -- --ignored --nocapture
```

Expected: PASS — ≥2 segment files under `${TMPDIR}/nvr-recorder-smoke`, each non-empty, monotonic start times. Optionally probe one:
`LD_LIBRARY_PATH=$PWD/ffmpeg/lib ./ffmpeg/bin/ffprobe -v error -show_format <file>.ts` returns a valid duration.

- [ ] **Step 4: Commit**

```bash
cargo fmt -p nvr-recorder
git add crates/nvr-recorder/tests
git commit -m "test(nvr-recorder): ignored smoke test against a live RTSP source"
```

---

## Notes for the implementer

- **RTSP timeout option:** we pass `timeout` (microseconds) on the input dictionary. On some FFmpeg builds the RTSP demuxer honors `stimeout` instead; if connects hang forever, add `stimeout` alongside `timeout`.
- **Negative timestamps:** audio packets slightly preceding the video keyframe that opened a segment get clamped to 0 in `SegmentWriter::write` — expected, keeps the muxer happy.
- **MP4 crash-safety:** only the currently-open segment is at risk on a crash (its `moov` is written at `finish()`); TS (the default) avoids this entirely.
- **Do not** wire this into `nvr-db`/`nvr`; mapping `SegmentInfo` onto `record_segment` is a separate, later effort (out of scope per the spec).
