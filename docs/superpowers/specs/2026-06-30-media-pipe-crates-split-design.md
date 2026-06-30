# Design: split `nvr/src/media` into reusable crates

Date: 2026-06-30

## Goal

Extract `nvr`'s `media` module into standalone, reusable crates so other
projects (e.g. media-srv / aibox-nvr) can depend on the media pipeline without
pulling in ZLMediaKit. The core crate must have **zero** `rszlm` dependency.

## Crate layout (under `crates/`)

### `media-pipe-core` (no rszlm)
- Config types: `PipeConfig`, `InputConfig`, `OutputConfig`, `OutputDest`,
  `EncodeConfig`, `VideoRawFrame` (from `types.rs`).
- `Pipe` (from `pipe.rs`, ZLM code removed) — drives `ffmpeg-bus`, forwards each
  output.
- `RawSinkSource` + stream adapters (from `stream.rs`).
- `to_fb_output` conversion to `ffmpeg-bus`.
- The `DemuxedSink` trait — the boundary for raw/demuxed passthrough sinks.
- Deps: `ffmpeg-bus`, tokio, futures, bytes, log, anyhow.

### `media-pipe-zlm` (depends on `media-pipe-core` + `rszlm`)
- `ZlmSink` implements `DemuxedSink` — holds `Arc<rszlm::media::Media>`, av type,
  shared coordinator.
- `ZlmTrackCoordinator` + the rszlm `Track`/`Frame` forwarding (today's
  `forward_raw_packet_stream_to_zlm`).
- A builder so `nvr` can make a coordinator + per-track sinks for one `Media`.

## Boundary (in core)

```rust
pub trait DemuxedSink: Send + Sync + 'static {
    fn start(&self, av: ffmpeg_bus::stream::AvStream,
             stream: ffmpeg_bus::bus::VideoRawFrameStream)
             -> tokio::task::JoinHandle<()>;
}

pub enum OutputDest {
    Network { url: String, format: String },
    RawFrame { sink: Arc<RawSinkSource> },
    RawPacket { sink: Arc<RawSinkSource> },
    Demuxed { sink: Arc<dyn DemuxedSink> }, // replaces Zlm(Arc<Media>)
}
```

`Arc<dyn DemuxedSink>` (not `Box`) keeps `OutputConfig: Clone`. `Demuxed` maps to
`FbOutputDest::Demuxed`, exactly as `Zlm` does today. The `Pipe` calls
`sink.start(av, stream)` per accepted Demuxed output and collects the
`JoinHandle`.

## Coordinator ownership

The `ZlmTrackCoordinator` (which gates video+audio track init on one `Media`)
moves out of the Pipe into the wiring layer: `nvr` builds the coordinator via
`media-pipe-zlm` and shares it across the `ZlmSink`s of one `Media`. The core
Pipe stays ZLM-agnostic.

## `nvr` wiring

- Depends on `media-pipe-core` always; `media-pipe-zlm` behind the existing
  `zlm` feature (which also pulls `rszlm`).
- `init/device.rs` & `handler/media_pipe.rs`: where they build
  `OutputDest::Zlm(media)` today, create the `Media` + coordinator and build
  `OutputDest::Demuxed { sink: Arc::new(ZlmSink::new(...)) }`.
- `manager.rs` uses `media_pipe_core::Pipe`.
- `nvr/src/media/` is deleted; imports become `media_pipe_core::...`.

## Migration steps (build-checked at each step)

1. Scaffold `crates/media-pipe-core`; move `types`/`stream`/`pipe` (strip ZLM,
   add `DemuxedSink`); move `pipe_test.rs`.
2. Scaffold `crates/media-pipe-zlm`; move ZLM forwarding + coordinator; impl
   `DemuxedSink`.
3. Rewire `nvr` (imports, `ZlmSink` wiring, feature flag); delete
   `nvr/src/media/`.
4. Workspace `Cargo.toml` members + `[workspace.dependencies]`; `cargo build
   --workspace` + `cargo test`.

## Testing

- `pipe_test.rs` moves to `media-pipe-core` (it is ZLM-agnostic — uses
  `RawSinkSource`).
- Add a small `media-pipe-zlm` test that `ZlmSink` satisfies `DemuxedSink`.
- `cargo build --workspace` + `cargo test --workspace`.
