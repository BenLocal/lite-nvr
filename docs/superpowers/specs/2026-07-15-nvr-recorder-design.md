# RTSP segment recorder crate (`nvr-recorder`)

Date: 2026-07-15
Status: approved (library + example; configurable tracks; fixed-duration +
optional wall-clock alignment; in-process ffmpeg-bus engine; default TS
container; reconnect on by default)

## Problem

The workspace has no standalone recorder that pulls an RTSP stream and writes
it to disk as time-sliced files. Recording today only happens as a side effect
of ZLM inside `nvr`. We want a small, reusable library crate that records one
RTSP source into rotating segment files (stream-copy, no transcode), with
accurate per-segment start/end timestamps and file metadata, plus a runnable
example to exercise it end to end. It must not modify `ffmpeg-bus` or wire into
`nvr`.

## Building blocks (already in `ffmpeg-bus`, unchanged)

- `input::AvInput::new(url, format, options)` — opens RTSP via
  `input_with_dictionary`; RTSP transport / socket timeout are passed as a
  `Dictionary` (`rtsp_transport=tcp`, `timeout=<microseconds>`).
- `input::AvInputTask` — spawns a blocking read loop and broadcasts
  `RawPacketCmd::Data(RawPacket)` / `RawPacketCmd::EOF`; `subscribe()` returns a
  receiver.
- `packet::RawPacket` — `.index()`, `.is_key()`, `.pts()`, `.dts()`,
  `.time_base()`, `.size()`, and `.get_mut()` for in-place timestamp edits.
- `stream::AvStream` — `.is_video()`, `.is_audio()`, `.parameters()`,
  `.time_base()`, `.width()/.height()`.
- `output::AvOutput` — `new(url, format, opts)`, `add_stream(&AvStream)`
  (stream-copy: copies codec parameters, no encoder), `write_packet(input_idx,
  RawPacket)` (rescales timestamps and enforces monotonically increasing DTS),
  `finish()` (writes trailer).

The recorder is pure orchestration on top of these — no libav code of its own
beyond in-place PTS/DTS offsetting via `RawPacket::get_mut()`.

## Crate layout

New workspace member `crates/nvr-recorder` (added to root `Cargo.toml`
`members`).

```
crates/nvr-recorder/
  Cargo.toml            # ffmpeg-bus, tokio, tokio-util, anyhow, chrono, log, serde
  src/
    lib.rs              # re-exports: RecorderConfig, Recorder, SegmentInfo, enums
    config.rs           # RecorderConfig + enums (RtspTransport, TrackSelect, Container)
    info.rs             # SegmentInfo, VideoMeta, AudioMeta
    rotation.rs         # pure rotation-decision + wall-clock-boundary logic
    segment.rs          # SegmentWriter — one output file over an AvOutput
    recorder.rs         # Recorder — the engine loop
  examples/
    record.rs           # CLI: one RTSP -> segment files + manifest.jsonl
```

## Public API

```rust
pub enum RtspTransport { Tcp, Udp }          // default Tcp
pub enum TrackSelect { Video, Audio, Both }  // default Both
pub enum Container { Ts, Mp4, Mkv }          // default Ts

pub struct RecorderConfig {
    pub url: String,
    pub transport: RtspTransport,
    pub tracks: TrackSelect,
    pub segment_time: Duration,       // per-segment target length
    pub align_to_wall_clock: bool,    // rotate on epoch-aligned boundaries
    pub container: Container,
    pub output_dir: PathBuf,
    pub filename_pattern: String,     // strftime of segment start wall-clock;
                                      // default "rec_%Y%m%d_%H%M%S"
    pub open_timeout: Duration,       // RTSP socket timeout
    pub reconnect: ReconnectPolicy,   // backoff; retries=0 disables
}

pub struct ReconnectPolicy {
    pub max_retries: Option<u32>,     // None = reconnect forever (until cancel);
                                      // Some(0) = no reconnect, exit on first EOF
    pub base_delay: Duration,         // e.g. 1s
    pub max_delay: Duration,          // e.g. 16s
}

impl RecorderConfig {
    /// Constructor with the documented defaults; caller overrides fields.
    /// Defaults: transport=Tcp, tracks=Both, segment_time=60s,
    /// align_to_wall_clock=false, container=Ts,
    /// filename_pattern="rec_%Y%m%d_%H%M%S", open_timeout=5s,
    /// reconnect={ max_retries: None, base_delay: 1s, max_delay: 16s }.
    pub fn new(url: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self;
}

pub struct Recorder { /* … */ }

impl Recorder {
    /// Build a recorder and the channel on which completed segments arrive.
    pub fn new(config: RecorderConfig) -> (Recorder, mpsc::Receiver<SegmentInfo>);
    /// Run until `cancel` fires or the stream ends and reconnect is exhausted.
    /// Each closed segment is sent on the mpsc receiver as it finishes.
    pub async fn run(self, cancel: CancellationToken) -> anyhow::Result<()>;
}
```

## Segment metadata (`info.rs`)

```rust
pub struct SegmentInfo {
    pub path: PathBuf,
    pub start_wall: DateTime<Utc>,   // segment start wall-clock (== filename time)
    pub end_wall: DateTime<Utc>,     // start_wall + measured duration
    pub duration: f64,               // measured seconds (last_pts - first_pts)
    pub size_bytes: u64,
    pub video: Option<VideoMeta>,    // codec, width, height, fps
    pub audio: Option<AudioMeta>,    // codec, sample_rate, channels
}
pub struct VideoMeta { pub codec: String, pub width: u32, pub height: u32, pub fps: f64 }
pub struct AudioMeta { pub codec: String, pub sample_rate: u32, pub channels: u32 }
```

The field set intentionally lines up with `nvr-db`'s `record_segment` schema so
a later "write recordings to the DB" step is a straight field map. That
integration is **out of scope** here; the library only emits `SegmentInfo`.

## Engine loop (`recorder.rs`)

1. Build the RTSP option `Dictionary` from `transport` + `open_timeout`; open
   `AvInput`. Enumerate `streams()`; resolve the target video and/or audio input
   stream indices per `tracks` (ignore tracks not selected; error if a selected
   kind is absent).
2. Start `AvInputTask(input)`, `subscribe()`.
3. Maintain a single **current `SegmentWriter`** (an `AvOutput` for one file)
   plus the selected `AvStream`s. For each `RawPacketCmd`:
   - **Data(pkt)** from a non-selected stream → drop.
   - **First segment gating:** if video is selected, do not open file #1 until
     the first *video keyframe* arrives (a stream-copy that begins mid-GOP is
     unplayable). Audio-only: open on the first audio packet.
   - **Rotation decision** (pure function in `rotation.rs`, see below): if it
     says rotate, `finish()` the current writer, emit its `SegmentInfo` on the
     mpsc, then open a new `SegmentWriter`.
   - Write the packet to the current writer.
   - **EOF** → finish + emit the current segment, then go to reconnect.
4. **Reconnect:** on EOF (drop/timeout), if reconnect is enabled
   (`max_retries` is `None` or `Some(n>0)`), reopen the input with exponential
   backoff (`base_delay`→…→`max_delay`, ×2, jittered), reusing `output_dir`;
   new files get fresh strftime names so nothing collides. `max_retries: None`
   retries forever (only `cancel` stops it — the default for a long-running
   recorder); a finite cap or `Some(0)` returns `Ok(())` once exhausted.
5. **Cancel:** `cancel` fires → stop the input task, finish + emit the current
   segment, return `Ok(())`.

## Rotation logic (`rotation.rs`, pure & unit-tested)

Inputs per candidate packet: whether it is a video keyframe, its wall-clock
arrival time, the segment's start wall-clock, the segment's elapsed media time,
`segment_time`, `align_to_wall_clock`, and whether video is being recorded.

- **video present:** rotation may only happen *on a video keyframe*. Given a
  keyframe, rotate when either
  - `align_to_wall_clock == false` and `elapsed_media >= segment_time`, or
  - `align_to_wall_clock == true` and `now >= next_boundary(segment_start,
    segment_time)`, where `next_boundary` is the next epoch multiple of
    `segment_time` strictly after the segment start.
- **audio-only:** identical, minus the keyframe gate (any audio packet may
  rotate).

Wall-clock alignment makes the **first** segment naturally short (it runs until
the first boundary); subsequent segments are full length. The boundary math and
the rotate/no-rotate decision are pure functions tested with synthetic inputs.

## Per-segment timestamp reset (`segment.rs`)

Each new file should start near PTS/DTS 0 so it is independently playable while
preserving A/V sync — emulating ffmpeg `-reset_timestamps 1`. When a
`SegmentWriter` opens, it records a single **base** = the DTS (fallback PTS) of
the packet that triggered the segment, expressed in `AV_TIME_BASE` microseconds.
For every packet written, it subtracts `rescale(base_us, AV_TIME_BASE_Q,
stream_time_base)` from the packet's PTS and DTS via `RawPacket::get_mut()`
before handing it to `AvOutput::write_packet`. Using one common base (rescaled
per stream) rather than a per-stream first value keeps video and audio aligned.

The writer also tracks first/last PTS (in stream time base → seconds) to compute
`duration`, accumulates `size_bytes` from `RawPacket::size()`, and reads codec /
resolution / sample-rate from the selected `AvStream` parameters to fill
`SegmentInfo` on `finish()`.

`AvOutput` for TS is `AvOutput::new(path, Some("mpegts"), None)`; Mp4 →
`Some("mp4")`; Mkv → `Some("matroska")`. TS default: crash-tolerant (a truncated
open segment still plays to the cut), passes H.264/H.265/AAC/G.711 through, and
matches this repo's existing `.ts` playback path.

## Example (`examples/record.rs`)

CLI (simple `std::env::args` or `clap` if already a workspace dep — otherwise
hand-parsed to avoid new deps): `--url`, `--dir`, `--segment-time`, `--tracks`,
`--container`, `--align`. Builds a `RecorderConfig`, spawns `Recorder::run` with
a `CancellationToken` wired to Ctrl-C, and consumes the `mpsc<SegmentInfo>`,
appending each record as one JSON line to `<dir>/manifest.jsonl` and logging it.
The library writes no manifest itself — manifest/DB policy stays in the caller.

## Error handling

- Selected-but-absent track (e.g. `Video` requested, source audio-only) → hard
  error before recording starts.
- Output open / write errors on a segment end the current session (a partial
  TS file remains playable) and are retried under `ReconnectPolicy`, the same
  path as RTSP open failure, EOF, and a lagged/closed input channel.
- Caveat: with the default `max_retries: None`, a permanent error (e.g. an
  unwritable `output_dir` or a source codec with no registered encoder) retries
  indefinitely; a caller that wants fail-fast behavior should set a finite
  `max_retries` (or `Some(0)`).

## Testing

- **Unit (no network, no ffmpeg):**
  - `rotation.rs`: rotate/no-rotate across duration-only, wall-clock-aligned,
    keyframe-gated, and audio-only cases; `next_boundary` math.
  - filename strftime formatting from a fixed `DateTime`.
  - timestamp-reset offset math (base rescale per stream time base).
- **Smoke (manual / `#[ignore]`):** point `examples/record.rs` at the repo's
  `examples/dummy-rtsp-camera` (oddity-rtsp-server), record ~35 s at 10 s
  segments, assert ≥3 files, a `manifest.jsonl` with monotonically increasing
  `start_wall`, and each file independently probeable by ffprobe.

## Out of scope (deliberate)

- Writing recordings into `nvr-db` `record_segment` / any `nvr` REST wiring.
- Multi-source supervision / scheduling (one recorder = one source here).
- Transcoding (stream-copy only; codecs a container can't hold are a config
  error, not an auto-transcode).
- Retention / cleanup of old segments (nvr already has a cleanup path).
