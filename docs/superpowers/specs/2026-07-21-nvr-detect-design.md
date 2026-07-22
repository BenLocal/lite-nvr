# Real-time multi-model object detection (nvr-detect)

Date: 2026-07-21
Status: approved (real-time frame tap; usls/ort backend behind a `Detector`
trait; same sampled frame fanned out to N YOLO models concurrently; per-model
results surfaced as structured JSON over REST; no server-side overlay /
re-encode this iteration)

## Problem

lite-nvr ingests camera video but has no object-detection stage. We want to run
YOLO detection on a live pipe and — crucially — **compare several models on the
exact same frame**: feed one sampled frame to N models at once and surface each
model's detections (label, bbox, confidence) plus its inference time, so the
models can be compared side by side in real time.

The reusable core is a backend-agnostic "given an image, run a model, get a
unified detection list" component. The real-time integration taps a running
pipe's decoded frames and drives that component.

## Why usls behind a trait

Different YOLO generations use different output tensor layouts and pre/post-
processing (v5/v7 carry objectness in `[n, 85]`; v8/v9/v10/v11 use `[84, n]`
without it). `usls` (built on `ort` = ONNX Runtime) already encapsulates the
pre/post-processing for YOLOv5/v8/v9/v11, RT-DETR and more, so multi-version
comparison — our core use case — costs us almost no glue. We still hide it
behind a `Detector` trait so an `ort`-only or `candle` backend can be added
later without touching callers. GPU is available through `ort` execution
providers (CUDA/TensorRT/CoreML) behind a cargo feature; the default is CPU.

## Scope (this iteration)

In scope:

- `crates/nvr-detect` library: `Detector` trait, unified detection types,
  `UslsDetector`, a synchronous `DetectorSet` fan-out, and an offline
  `detect-compare` example.
- `nvr/src/detect` integration: per-pipe tap, hub, frame conversion, REST API,
  mirroring the existing `nvr/src/asr` real-time pattern.
- A small, symmetric `subscribe_video` addition to `ffmpeg-bus` (the only way to
  get decoded video frames off a running pipe today).

Explicitly **out of scope** (deferred, each its own future spec if wanted):

- Server-side box overlay + re-encode + re-push to ZLM (annotated stream).
- Push transport (SSE/WebSocket) — this iteration is poll-only.
- Object tracking, counting, zone/line rules, alarms.
- Multiple input sizes per model, model training/export.
- Dashboard UI for drawing boxes (frontend consumes the JSON later).

## The real-time pattern we mirror

`nvr/src/asr` already implements exactly this shape and is the template:

- `nvr-asr` (leaf crate) = the engine; `nvr/src/asr` = orchestration.
- `AsrHub`: a process-global `OnceLock`, lazy shared models, and a registry of
  running per-pipe tap tasks keyed by `CancellationToken`.
- `asr::tap::run(pipe, models, receiver, sink, cancel)`: loops over a decoded-
  frame broadcast (`RawFrameReceiver` of `RawFrameCmd::Data(RawFrame::…)`),
  processes, emits.
- `asr::api`: `POST /api/asr/{pipe}/start` / `/stop`, session-auth guarded.

Detection reuses this structure verbatim, swapping audio→video, and swapping the
Socket.IO push for a stored latest-result that a `GET` returns.

## Frame source: `subscribe_video`

`Bus::subscribe_audio_internal` (bus.rs) finds the **audio** stream, starts an
audio decoder, and returns that decoder's `subscribe()` broadcast. There is no
video equivalent today — the `RawFrame::Video(_)` arm in `asr::tap` is only
defensive.

Add a symmetric `subscribe_video` path:

- `Bus::subscribe_video_internal(state)`: find the first `is_video()` input
  stream, `start_decoder_task(state, video_index, …)`, return that decoder task's
  `.subscribe()` (`RawFrameReceiver`), then ensure the input task is running —
  identical structure to the audio version.
- `Pipe::subscribe_video(&self) -> anyhow::Result<RawFrameReceiver>` public
  wrapper, mirroring `Pipe::subscribe_audio`.

If the pipe is stream-copy (no decoder running), this starts a **new** video
decoder solely for detection — the same on-demand behavior ASR uses for audio.
The broadcast yields `RawFrameCmd::Data(RawFrame::Video(frame))` where `frame`
exposes `width()`, `height()`, `format() -> Pixel`, and `data()`.

## Architecture

### Unit 1 — `crates/nvr-detect` (backend-agnostic library)

Files:

- `types.rs` — unified output:
  - `struct BBox { x1: f32, y1: f32, x2: f32, y2: f32 }` (original-frame pixels).
  - `struct Detection { class_id: usize, label: String, bbox: BBox, confidence: f32 }`.
  - `struct ModelResult { name: String, infer_ms: f64, detections: Vec<Detection>, error: Option<String> }`.
- `config.rs` — `struct DetectorConfig { name: String, model_path: PathBuf,
  version: YoloVersion, task: Task, input_size: u32, conf: f32, iou: f32,
  class_names: Vec<String>, device: Device }`; serde-deserializable; a
  `models.json` is `Vec<DetectorConfig>` (paths relative to `DETECT_MODELS_DIR`).
- `detector.rs` — `trait Detector: Send + Sync { fn name(&self) -> &str;
  fn detect(&self, img: &image::RgbImage) -> anyhow::Result<Vec<Detection>>; }`.
- `usls_backend.rs` — `UslsDetector` wrapping `usls::YOLO`, built from a
  `DetectorConfig`; `detect` converts the `RgbImage` into usls's input, runs
  inference, and maps usls detections into `Vec<Detection>` (label from
  `class_names`, bbox in pixels, confidence). All pre/post-processing is usls's.
  The trait method is `detect(&self, …)` so a detector shares as `Arc<dyn
  Detector>`; if usls inference needs `&mut self`, `UslsDetector` holds its
  `usls::YOLO` behind an internal `Mutex` and `detect(&self)` locks it.
  Contention is nil — each detector is invoked at most once per sampled frame.
- `set.rs` — `struct DetectorSet { detectors: Vec<Box<dyn Detector>> }` with
  `fn detect_all(&self, img: &image::RgbImage) -> Vec<ModelResult>` (synchronous,
  sequential; each model timed; a model error becomes `ModelResult.error` rather
  than failing the batch). Used by the offline example and tests; the real-time
  path runs the same detectors concurrently (below).
- `lib.rs` — re-exports.
- `examples/detect-compare.rs` — CLI: `--image <path> --models <models.json>`;
  builds a `DetectorSet`, runs `detect_all`, prints a comparison table
  (model, box count, mean confidence, infer_ms). This validates usls and the
  unified output **before** any pipe wiring.

Why a separate crate: same rationale as `nvr-asr` — the inference engine is
runtime-agnostic and independently testable; orchestration lives in `nvr`.

### Unit 2 — `nvr/src/detect` (real-time integration)

Files:

- `mod.rs` — module wiring; `fn model_config() -> Vec<DetectorConfig>` resolving
  `DETECT_MODELS_DIR` (default `third_party/detect-models`) + `models.json`.
- `hub.rs` — `DetectHub` (process-global `OnceLock`), mirroring `AsrHub`:
  - lazy shared `Vec<Arc<dyn Detector>>` built from `model_config()` (heavy first
    call);
  - `running: Mutex<HashMap<pipe, CancellationToken>>` — `register` / `unregister`
    / `is_running`;
  - `latest: Mutex<HashMap<pipe, FrameResult>>` — `store(pipe, FrameResult)` /
    `latest(pipe) -> Option<FrameResult>` (replaces ASR's Socket.IO push).
- `convert.rs` — `to_rgb(frame) -> anyhow::Result<image::RgbImage>` over the
  `RawFrame::Video` payload (decoded video frame exposing `width()`/`height()`/
  `format() -> Pixel`/`data()`): scale/convert its pixel format (e.g. YUV420P) to
  RGB24 via the existing `ffmpeg-bus` scaler, then wrap the RGB24 bytes as an
  `RgbImage`. Confirm the exact payload type name against `ffmpeg-bus::frame`
  during implementation.
- `tap.rs` — `async fn run(pipe, detectors: Vec<Arc<dyn Detector>>,
  mut video: RawFrameReceiver, hub, cancel)`:
  - loop with `tokio::select!` on `cancel` and `video.recv()`;
  - **sample**: keep a `last_sampled: Instant`; if a frame arrives less than
    `sample_interval_ms` after the last sample, drop it (never queue). Tolerate
    `RecvError::Lagged` by continuing. Detection is the bottleneck — only the
    newest frame past the interval is processed;
  - convert to `RgbImage`; fan out: for each detector, `tokio::task::spawn_blocking`
    (usls/ort inference is blocking CPU) timing each; `join` all into
    `Vec<ModelResult>`;
  - build `FrameResult { ts, frame_w, frame_h, models }` and `hub.store(pipe, …)`.
- `api.rs` — session-auth-guarded routes (GET/POST only, matching project
  convention):
  - `POST /api/detect/{pipe}/start` — body: `{ models?: [name…] }` (subset of
    configured models; omitted = all). Builds/reuses shared detectors, spawns the
    tap, `hub.register`. Errors if the pipe has no video stream or a model fails
    to load; the tap is not registered on error.
  - `POST /api/detect/{pipe}/stop` — `hub.unregister` (cancels the tap).
  - `GET /api/detect/{pipe}/latest` — the stored `FrameResult` as JSON, or 404 if
    none yet.
  - `GET /api/detect/models` — the configured model list (names + versions).

`FrameResult` JSON shape:

```json
{ "ts": 1690000000, "frame_w": 1920, "frame_h": 1080,
  "models": [
    { "name": "yolov8n", "infer_ms": 12.3,
      "detections": [ { "class_id": 0, "label": "person",
        "bbox": { "x1": 34.0, "y1": 50.0, "x2": 220.0, "y2": 640.0 },
        "confidence": 0.82 } ] },
    { "name": "yolo11s", "infer_ms": 31.0, "detections": [ ] }
  ] }
```

Coordinates are original-frame pixels; `frame_w`/`frame_h` let a consumer scale
boxes to whatever size it renders.

## Data flow

```
RTSP → pipe → (on-demand) video decoder → RawFrame::Video broadcast
                                                │ subscribe_video
                                                ▼
                          tap: sample (~2fps) → YUV→RGB (convert.rs)
                                                │
                                    spawn_blocking fan-out
                              ┌──────────┼──────────┐
                           yolov8n    yolo11s    yolov5s   ← usls::YOLO × N
                              └──────────┼──────────┘  join → FrameResult
                                                ▼
                                hub.store(pipe, FrameResult)
                                                ▲ GET /api/detect/{pipe}/latest
                                           consumer (dashboard canvas, later)
```

## Configuration & models

- `DETECT_MODELS_DIR` (default `third_party/detect-models`) holds ONNX weights
  and a `models.json` manifest (`Vec<DetectorConfig>`, model paths relative to
  the dir). COCO-80 class names are the default `class_names`.
- Weights are **not** committed (same policy as ASR models); document a manual
  placement / download step.
- `sample_interval_ms` default 500 (~2 fps), configurable (start-request field or
  config default).
- GPU: default CPU. `ort` execution providers are behind a cargo feature
  (`cuda` / `tensorrt` / `coreml`); `DetectorConfig.device` selects, default CPU.

## Error handling

- Model load failure at `start` → the API returns an error; the tap is not
  registered.
- Per-frame, per-model `detect` error → logged; that model's `ModelResult` has
  empty `detections` and a populated `error`; **other models still report** and
  the tap keeps running.
- Pipe has no video stream / `subscribe_video` fails → `start` returns an error.
- `stop`/cancel → the tap ends; the last `FrameResult` is retained (a consumer
  can tell it is stale from `ts`).
- The video decoder is started on demand; if the input never produces video the
  tap simply never stores a result (and `GET /latest` stays 404).

## Testing

- **Library unit (pure, no network/GPU):**
  - `types`: `Detection`/`ModelResult` (de)serialize to the documented JSON.
  - `config`: a sample `models.json` parses into `Vec<DetectorConfig>` with the
    expected fields and defaults.
  - `set`: a `FakeDetector` (returns canned detections; one variant returns
    `Err`) drives `DetectorSet::detect_all` — asserts each model is timed,
    ordering preserved, and an erroring model yields a `ModelResult` with `error`
    set while the others still populate.
- **Library ignored live test:** with a real `yolov8n.onnx` present, load it and
  run `detect` on a bundled test image; assert it finds an expected class
  (mirrors `nvr-onvif`'s `#[ignore]` live test).
- **nvr integration:**
  - `convert`: a synthetic YUV420P frame converts to an `RgbImage` of the right
    dimensions.
  - Manual E2E against `dummy-rtsp-camera`: add a device, `POST /start` with two
    models, `GET /latest` shows both models' `ModelResult`s with detections.

## Conventions

Rust edition 2024; `cargo fmt`; snake_case; tests colocated as `*_test.rs` via
`#[cfg(test)] #[path = "…_test.rs"] mod …;`. API is GET/POST only and session-
auth guarded, consistent with the rest of `/api`. Reuse `ffmpeg-bus` primitives
(scaler, decoder subscribe) rather than hand-rolling media code.
