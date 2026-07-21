# nvr-detect: Real-time Multi-model Object Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tap a running pipe's decoded video, sample at ~2fps, fan one frame out to N YOLO models concurrently, and surface each model's detections (label/bbox/confidence/timing) as structured JSON over REST.

**Architecture:** A backend-agnostic library crate `nvr-detect` (a `Detector` trait, unified detection types, a usls-backed `UslsDetector`, a synchronous `DetectorSet` fan-out, and an offline `detect-compare` example) plus a `nvr/src/detect` real-time integration (per-pipe tap, hub, frame conversion, REST API) that mirrors the existing `nvr/src/asr` pattern. A small symmetric `subscribe_video` is added to `ffmpeg-bus`/`media-pipe-core` — the only way to get decoded video frames off a running pipe today.

**Tech Stack:** Rust edition 2024; `usls` 0.1.11 (ONNX Runtime YOLO inference); Axum 0.8; Tokio; serde; `ffmpeg-next` (scaler for YUV→RGB).

## Global Constraints

- Rust edition 2024; `cargo fmt`; snake_case modules/files/functions.
- Tests colocated as `<module>_test.rs`, imported via `#[cfg(test)] #[path = "<module>_test.rs"] mod <module>_test;` at the end of the source file.
- REST API is **GET/POST only** and lives under `/api`, which is already session-auth guarded by `require_auth` — do not add PUT/DELETE and do not add per-route auth.
- Reuse `ffmpeg-bus` primitives (`Scaler`, decoder `subscribe`) — do not hand-roll media/FFI code.
- The `Detector` trait takes an image as **raw RGB8 bytes + width + height** (`&[u8], u32, u32`), NOT `image::RgbImage`. This decouples the trait from both `usls` and the `image` crate and maps directly onto `usls::Image::from_u8s`. (This refines the spec's `image::RgbImage` mention; external behavior is unchanged.)
- Bounding-box coordinates in `Detection` are **original-frame pixels** (xyxy).
- `DETECT_MODELS_DIR` default is `third_party/detect-models`; models are loaded from a `models.json` manifest there; model weights are NOT committed to git.
- Default sampling interval is **500 ms** (~2 fps). Default device is **CPU**.
- `usls` 0.1.11's builder method names (`Config::yolo_detect`, `with_model_file`, `with_class_confs`, `with_class_names`, `with_model_device`, `with_model_ixx`, `commit`, `YOLO::new`, `YOLO::forward`, `Y::hbbs`, `Hbb::{xmin,ymin,xmax,ymax,id,name,confidence}`, `Image::from_u8s`) are taken from usls docs. Task 3 MUST verify them against the installed version with `cargo doc -p usls` (or docs.rs for 0.1.11) before writing `usls_backend.rs`, and adjust names if the installed API differs. The offline `detect-compare` example is sequenced first precisely to surface any usls API mismatch before the real-time wiring.

---

### Task 1: `nvr-detect` crate scaffold + unified types + config

**Files:**
- Modify: `Cargo.toml` (workspace `members`) — add `"crates/nvr-detect"`.
- Create: `crates/nvr-detect/Cargo.toml`
- Create: `crates/nvr-detect/src/lib.rs`
- Create: `crates/nvr-detect/src/types.rs`
- Test: `crates/nvr-detect/src/types_test.rs`
- Create: `crates/nvr-detect/src/config.rs`
- Test: `crates/nvr-detect/src/config_test.rs`
- Create: `crates/nvr-detect/src/coco.rs`

**Interfaces:**
- Produces:
  - `BBox { x1: f32, y1: f32, x2: f32, y2: f32 }` (serde `Serialize`/`Deserialize`, `Clone`, `Debug`, `PartialEq`).
  - `Detection { class_id: usize, label: String, bbox: BBox, confidence: f32 }` (same derives).
  - `ModelResult { name: String, infer_ms: f64, detections: Vec<Detection>, error: Option<String> }` (same derives).
  - `DetectorConfig { name, model_file, version, scale, input_size, conf, iou, class_names, device }` (`Deserialize`, `Clone`, `Debug`); field types in Step 3.
  - `coco::COCO_80: [&str; 80]` and `coco::default_names() -> Vec<String>`.

- [ ] **Step 1: Add the crate to the workspace and write its Cargo.toml**

In `Cargo.toml`, add `"crates/nvr-detect",` to the `members` array (next to `"crates/nvr-onvif",`).

Create `crates/nvr-detect/Cargo.toml`:

```toml
[package]
name = "nvr-detect"
version = "0.1.0"
edition = "2024"
publish = false
description = "Backend-agnostic object detection: a Detector trait, unified detection types, and a usls/ONNX-Runtime YOLO backend. Runs one image through N models and returns each model's detections (label, bbox, confidence, timing)."

[dependencies]
anyhow = { workspace = true }
log = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
```

(`usls` and the `detect-compare` example are added in Task 3; Tasks 1–2 are pure and must not pull the heavy ONNX dep.)

- [ ] **Step 2: Write the failing test for the detection types**

Create `crates/nvr-detect/src/types_test.rs`:

```rust
use super::*;

#[test]
fn detection_serializes_to_documented_shape() {
    let d = Detection {
        class_id: 0,
        label: "person".to_string(),
        bbox: BBox { x1: 34.0, y1: 50.0, x2: 220.0, y2: 640.0 },
        confidence: 0.82,
    };
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["class_id"], 0);
    assert_eq!(v["label"], "person");
    assert_eq!(v["bbox"]["x1"], 34.0);
    assert_eq!(v["bbox"]["y2"], 640.0);
    assert_eq!(v["confidence"], 0.82);
}

#[test]
fn model_result_roundtrips() {
    let m = ModelResult {
        name: "yolov8n".to_string(),
        infer_ms: 12.3,
        detections: vec![],
        error: None,
    };
    let s = serde_json::to_string(&m).unwrap();
    let back: ModelResult = serde_json::from_str(&s).unwrap();
    assert_eq!(back.name, "yolov8n");
    assert_eq!(back.detections.len(), 0);
    assert!(back.error.is_none());
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p nvr-detect --lib`
Expected: FAIL to compile (`types` module / `Detection` not found).

- [ ] **Step 4: Implement `types.rs`, `coco.rs`, `config.rs`, and `lib.rs`**

Create `crates/nvr-detect/src/types.rs`:

```rust
//! Unified detection output — identical shape regardless of which model or
//! backend produced it.

use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box in original-frame pixel coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// One detected object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub class_id: usize,
    pub label: String,
    pub bbox: BBox,
    pub confidence: f32,
}

/// One model's result for a single frame, with its inference time. `error` is
/// set (and `detections` empty) if that model failed on this frame; other
/// models in the same batch are unaffected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelResult {
    pub name: String,
    pub infer_ms: f64,
    pub detections: Vec<Detection>,
    pub error: Option<String>,
}

#[cfg(test)]
#[path = "types_test.rs"]
mod types_test;
```

Create `crates/nvr-detect/src/coco.rs`:

```rust
//! The 80 COCO class names, in model-output order. Used as the default label
//! set when a `DetectorConfig` supplies no explicit `class_names`.

pub const COCO_80: [&str; 80] = [
    "person", "bicycle", "car", "motorcycle", "airplane", "bus", "train",
    "truck", "boat", "traffic light", "fire hydrant", "stop sign",
    "parking meter", "bench", "bird", "cat", "dog", "horse", "sheep", "cow",
    "elephant", "bear", "zebra", "giraffe", "backpack", "umbrella", "handbag",
    "tie", "suitcase", "frisbee", "skis", "snowboard", "sports ball", "kite",
    "baseball bat", "baseball glove", "skateboard", "surfboard",
    "tennis racket", "bottle", "wine glass", "cup", "fork", "knife", "spoon",
    "bowl", "banana", "apple", "sandwich", "orange", "broccoli", "carrot",
    "hot dog", "pizza", "donut", "cake", "chair", "couch", "potted plant",
    "bed", "dining table", "toilet", "tv", "laptop", "mouse", "remote",
    "keyboard", "cell phone", "microwave", "oven", "toaster", "sink",
    "refrigerator", "book", "clock", "vase", "scissors", "teddy bear",
    "hair drier", "toothbrush",
];

/// COCO-80 names as owned `String`s.
pub fn default_names() -> Vec<String> {
    COCO_80.iter().map(|s| s.to_string()).collect()
}
```

Create `crates/nvr-detect/src/config.rs`:

```rust
//! Per-model configuration. A `models.json` manifest is a JSON array of these.

use serde::Deserialize;

fn default_input_size() -> u32 {
    640
}
fn default_conf() -> f32 {
    0.25
}
fn default_iou() -> f32 {
    0.45
}
fn default_device() -> String {
    "cpu".to_string()
}

/// One model's configuration. `model_file` is resolved relative to
/// `DETECT_MODELS_DIR` by the loader (see `nvr::detect::model_config`).
#[derive(Clone, Debug, Deserialize)]
pub struct DetectorConfig {
    /// Display name shown in results (e.g. "yolov8n").
    pub name: String,
    /// Path to the `.onnx` weights (relative to the models dir).
    pub model_file: String,
    /// Optional usls YOLO version hint (e.g. 8.0, 11.0). None = let usls infer.
    #[serde(default)]
    pub version: Option<f32>,
    /// Optional usls scale hint ("n"/"s"/"m"/"l"/"x").
    #[serde(default)]
    pub scale: Option<String>,
    /// Square model input size (pixels).
    #[serde(default = "default_input_size")]
    pub input_size: u32,
    /// Confidence threshold.
    #[serde(default = "default_conf")]
    pub conf: f32,
    /// IoU / NMS threshold.
    #[serde(default = "default_iou")]
    pub iou: f32,
    /// Class names in model-output order. Empty = default to COCO-80.
    #[serde(default)]
    pub class_names: Vec<String>,
    /// Inference device: "cpu" or e.g. "cuda:0".
    #[serde(default = "default_device")]
    pub device: String,
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
```

Create `crates/nvr-detect/src/lib.rs`:

```rust
//! Backend-agnostic object detection.

pub mod coco;
pub mod config;
pub mod types;

pub use config::DetectorConfig;
pub use types::{BBox, Detection, ModelResult};
```

- [ ] **Step 5: Write the failing test for config parsing**

Create `crates/nvr-detect/src/config_test.rs`:

```rust
use super::*;

#[test]
fn parses_manifest_with_defaults() {
    let json = r#"[
      { "name": "yolov8n", "model_file": "yolov8n.onnx", "version": 8.0 },
      { "name": "yolo11s", "model_file": "yolo11s.onnx", "input_size": 640,
        "conf": 0.3, "device": "cpu" }
    ]"#;
    let cfgs: Vec<DetectorConfig> = serde_json::from_str(json).unwrap();
    assert_eq!(cfgs.len(), 2);
    // Defaults applied on the first entry.
    assert_eq!(cfgs[0].input_size, 640);
    assert_eq!(cfgs[0].conf, 0.25);
    assert_eq!(cfgs[0].device, "cpu");
    assert!(cfgs[0].class_names.is_empty());
    assert_eq!(cfgs[0].version, Some(8.0));
    // Explicit values on the second.
    assert_eq!(cfgs[1].conf, 0.3);
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p nvr-detect --lib`
Expected: PASS (4 tests: 2 in types_test, 1 in config_test, plus compile). Also run `cargo fmt` and `cargo build -p nvr-detect`.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/nvr-detect
git commit -m "feat(nvr-detect): crate scaffold, unified detection types, config"
```

---

### Task 2: `Detector` trait + `DetectorSet` fan-out

**Files:**
- Create: `crates/nvr-detect/src/detector.rs`
- Create: `crates/nvr-detect/src/set.rs`
- Test: `crates/nvr-detect/src/set_test.rs`
- Modify: `crates/nvr-detect/src/lib.rs` (export the new modules)

**Interfaces:**
- Consumes: `Detection`, `ModelResult` (Task 1).
- Produces:
  - `trait Detector: Send + Sync { fn name(&self) -> &str; fn detect(&self, rgb: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<Detection>>; }`
  - `struct DetectorSet { detectors: Vec<Box<dyn Detector>> }` with `fn new(detectors: Vec<Box<dyn Detector>>) -> Self` and `fn detect_all(&self, rgb: &[u8], width: u32, height: u32) -> Vec<ModelResult>` (sequential, each model timed, an `Err` becomes `ModelResult.error`).

- [ ] **Step 1: Write the failing test**

Create `crates/nvr-detect/src/set_test.rs`:

```rust
use super::*;
use crate::types::{BBox, Detection};

struct FakeDetector {
    name: String,
    out: anyhow::Result<Vec<Detection>>,
}

impl Detector for FakeDetector {
    fn name(&self) -> &str {
        &self.name
    }
    fn detect(&self, _rgb: &[u8], _w: u32, _h: u32) -> anyhow::Result<Vec<Detection>> {
        match &self.out {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(anyhow::anyhow!("{e:#}")),
        }
    }
}

fn det(label: &str) -> Detection {
    Detection {
        class_id: 0,
        label: label.to_string(),
        bbox: BBox { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 },
        confidence: 0.9,
    }
}

#[test]
fn detect_all_preserves_order_times_each_and_captures_errors() {
    let set = DetectorSet::new(vec![
        Box::new(FakeDetector { name: "a".into(), out: Ok(vec![det("person")]) }),
        Box::new(FakeDetector { name: "b".into(), out: Err(anyhow::anyhow!("boom")) }),
    ]);
    let rgb = vec![0u8; 3]; // 1x1 RGB
    let results = set.detect_all(&rgb, 1, 1);

    assert_eq!(results.len(), 2);
    // Order preserved.
    assert_eq!(results[0].name, "a");
    assert_eq!(results[1].name, "b");
    // Success path.
    assert_eq!(results[0].detections.len(), 1);
    assert_eq!(results[0].detections[0].label, "person");
    assert!(results[0].error.is_none());
    // Error path: empty detections, error populated, other models unaffected.
    assert!(results[1].detections.is_empty());
    assert!(results[1].error.as_deref().unwrap().contains("boom"));
    // Every model is timed (>= 0.0).
    assert!(results[0].infer_ms >= 0.0);
    assert!(results[1].infer_ms >= 0.0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nvr-detect --lib set_test`
Expected: FAIL to compile (`Detector` / `DetectorSet` not found).

- [ ] **Step 3: Implement `detector.rs` and `set.rs`**

Create `crates/nvr-detect/src/detector.rs`:

```rust
//! The one interface every backend implements.

use crate::types::Detection;

/// A single object-detection model. Implementations own their own pre/post-
/// processing; callers hand raw RGB8 bytes and get back unified `Detection`s in
/// original-frame pixel coordinates.
///
/// `detect` takes `&self` so a detector can be shared as `Arc<dyn Detector>`
/// and invoked concurrently across models; a backend needing interior
/// mutability (e.g. an ONNX session) hides it behind its own lock.
pub trait Detector: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self, rgb: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<Detection>>;
}
```

Create `crates/nvr-detect/src/set.rs`:

```rust
//! A synchronous fan-out over several detectors: run one image through all of
//! them and collect each model's timed result. Used by the offline
//! `detect-compare` example and tests; the real-time path (nvr) runs the same
//! detectors concurrently.

use std::time::Instant;

use crate::detector::Detector;
use crate::types::ModelResult;

pub struct DetectorSet {
    detectors: Vec<Box<dyn Detector>>,
}

impl DetectorSet {
    pub fn new(detectors: Vec<Box<dyn Detector>>) -> Self {
        Self { detectors }
    }

    pub fn len(&self) -> usize {
        self.detectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.detectors.is_empty()
    }

    /// Run every detector on the same image, in order. Each model is timed; a
    /// failing model yields a `ModelResult` with empty detections and `error`
    /// set, leaving the others intact.
    pub fn detect_all(&self, rgb: &[u8], width: u32, height: u32) -> Vec<ModelResult> {
        self.detectors
            .iter()
            .map(|d| {
                let start = Instant::now();
                let res = d.detect(rgb, width, height);
                let infer_ms = start.elapsed().as_secs_f64() * 1000.0;
                match res {
                    Ok(detections) => ModelResult {
                        name: d.name().to_string(),
                        infer_ms,
                        detections,
                        error: None,
                    },
                    Err(e) => ModelResult {
                        name: d.name().to_string(),
                        infer_ms,
                        detections: vec![],
                        error: Some(format!("{e:#}")),
                    },
                }
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "set_test.rs"]
mod set_test;
```

Update `crates/nvr-detect/src/lib.rs` to export the modules:

```rust
//! Backend-agnostic object detection.

pub mod coco;
pub mod config;
pub mod detector;
pub mod set;
pub mod types;

pub use config::DetectorConfig;
pub use detector::Detector;
pub use set::DetectorSet;
pub use types::{BBox, Detection, ModelResult};
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nvr-detect --lib`
Expected: PASS. Run `cargo fmt`.

- [ ] **Step 5: Commit**

```bash
git add crates/nvr-detect/src
git commit -m "feat(nvr-detect): Detector trait and DetectorSet fan-out"
```

---

### Task 3: `UslsDetector` backend + `detect-compare` example

**Files:**
- Modify: `crates/nvr-detect/Cargo.toml` (add `usls`; add `image` + `clap` as example deps; declare the example)
- Create: `crates/nvr-detect/src/usls_backend.rs`
- Modify: `crates/nvr-detect/src/lib.rs` (export `UslsDetector`)
- Create: `crates/nvr-detect/examples/detect-compare.rs`
- Test: `crates/nvr-detect/tests/live.rs` (ignored, loads a real model)

**Interfaces:**
- Consumes: `Detector`, `DetectorConfig`, `Detection`, `BBox`, `coco::default_names`.
- Produces: `UslsDetector` implementing `Detector`, with `fn new(cfg: &DetectorConfig, model_path: &std::path::Path) -> anyhow::Result<Self>`.

- [ ] **Step 1: Verify the usls API, then add the dependency**

FIRST verify the builder/output names against the installed usls version:

Run: `cargo add usls@0.1.11 -p nvr-detect && cargo doc -p usls --no-deps`

Confirm these exist (adjust the code below if the installed 0.1.11 differs): `usls::Config::yolo_detect()`, `Config::with_model_file(&str)`, `Config::with_class_confs(Vec<f32>)`, `Config::with_class_names(Vec<String>)`, `Config::with_model_device(&str)`, `Config::with_model_ixx(...)`, `Config::commit()`, `usls::models::YOLO::new(Config)`, `YOLO::forward(&[Image]) -> Result<Vec<Y>>`, `usls::Image::from_u8s(&[u8], u32, u32) -> Result<Image>`, `Y::hbbs() -> &[Hbb]`, `Hbb::{xmin,ymin,xmax,ymax}() -> f32`, `Hbb::id() -> Option<usize>`, `Hbb::name() -> Option<&str>`, `Hbb::confidence() -> Option<f32>`.

Then edit `crates/nvr-detect/Cargo.toml` to the final form:

```toml
[package]
name = "nvr-detect"
version = "0.1.0"
edition = "2024"
publish = false
description = "Backend-agnostic object detection: a Detector trait, unified detection types, and a usls/ONNX-Runtime YOLO backend. Runs one image through N models and returns each model's detections (label, bbox, confidence, timing)."

[dependencies]
anyhow = { workspace = true }
log = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
usls = "0.1.11"

[dev-dependencies]
image = "0.25"
clap = { version = "4", features = ["derive"] }
env_logger = { workspace = true }

[[example]]
name = "detect-compare"
path = "examples/detect-compare.rs"
```

- [ ] **Step 2: Write the ignored live test (the real usls smoke)**

Create `crates/nvr-detect/tests/live.rs`:

```rust
//! End-to-end smoke against a real ONNX model. Ignored by default; needs a
//! weights file. Run:
//!   DETECT_TEST_MODEL=third_party/detect-models/yolov8n.onnx \
//!   DETECT_TEST_IMAGE=crates/nvr-detect/tests/bus.jpg \
//!     cargo test -p nvr-detect --test live -- --ignored --nocapture

use nvr_detect::{DetectorConfig, UslsDetector, Detector};

#[test]
#[ignore]
fn detects_on_a_real_image() {
    let model = std::env::var("DETECT_TEST_MODEL").expect("set DETECT_TEST_MODEL");
    let image = std::env::var("DETECT_TEST_IMAGE").expect("set DETECT_TEST_IMAGE");

    let cfg = DetectorConfig {
        name: "yolo".into(),
        model_file: model.clone(),
        version: None,
        scale: None,
        input_size: 640,
        conf: 0.25,
        iou: 0.45,
        class_names: vec![],
        device: "cpu".into(),
    };
    let det = UslsDetector::new(&cfg, std::path::Path::new(&model)).expect("build detector");

    let img = image::open(&image).expect("open image").to_rgb8();
    let (w, h) = img.dimensions();
    let dets = det.detect(img.as_raw(), w, h).expect("detect");

    println!("found {} detections", dets.len());
    for d in &dets {
        println!("  {} {:.2} {:?}", d.label, d.confidence, d.bbox);
    }
    assert!(!dets.is_empty(), "expected at least one detection");
}
```

- [ ] **Step 3: Run it to confirm it is ignored (compile gate)**

Run: `cargo test -p nvr-detect --test live`
Expected: compile FAILS (`UslsDetector` not found) — this is the failing state that Step 4 fixes.

- [ ] **Step 4: Implement `usls_backend.rs`**

Create `crates/nvr-detect/src/usls_backend.rs`:

```rust
//! usls (ONNX Runtime) YOLO backend. Wraps `usls::models::YOLO` behind a
//! `Mutex` so `detect(&self)` satisfies the `Detector` trait even though usls
//! inference needs `&mut self`; contention is nil because each detector runs at
//! most once per sampled frame.

use std::path::Path;
use std::sync::Mutex;

use usls::models::YOLO;
use usls::{Config, Image};

use crate::coco;
use crate::config::DetectorConfig;
use crate::detector::Detector;
use crate::types::{BBox, Detection};

pub struct UslsDetector {
    name: String,
    model: Mutex<YOLO>,
}

impl UslsDetector {
    /// Build a detector from config. `model_path` is the resolved absolute path
    /// to the `.onnx` weights.
    pub fn new(cfg: &DetectorConfig, model_path: &Path) -> anyhow::Result<Self> {
        let path = model_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-utf8 model path"))?;

        let names = if cfg.class_names.is_empty() {
            coco::default_names()
        } else {
            cfg.class_names.clone()
        };
        let n = names.len();

        let mut config = Config::yolo_detect()
            .with_model_file(path)
            .with_class_confs(vec![cfg.conf; n])
            .with_class_names(names)
            .with_model_device(&cfg.device);
        // `version`/`scale` are optional usls hints. `input_size` and `iou` are
        // manifest hints: usls reads the real input dims from the ONNX model and
        // runs NMS internally, so they are advisory. If Step 1 confirms the
        // installed usls exposes `with_model_ixx` / `with_iou`, apply them here.
        log::debug!(
            "detector {}: conf={} iou={} input_size={} device={}",
            cfg.name, cfg.conf, cfg.iou, cfg.input_size, cfg.device
        );
        if let Some(v) = cfg.version {
            config = config.with_version(v);
        }
        if let Some(scale) = &cfg.scale {
            config = config.with_scale(scale.parse().unwrap_or_default());
        }
        let config = config.commit()?;

        let model = YOLO::new(config)?;
        Ok(Self {
            name: cfg.name.clone(),
            model: Mutex::new(model),
        })
    }
}

impl Detector for UslsDetector {
    fn name(&self) -> &str {
        &self.name
    }

    fn detect(&self, rgb: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<Detection>> {
        let image = Image::from_u8s(rgb, width, height)?;
        let mut model = self
            .model
            .lock()
            .map_err(|_| anyhow::anyhow!("detector mutex poisoned"))?;
        let ys = model.forward(&[image])?;
        let y = ys
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("usls returned no output"))?;

        let mut out = Vec::new();
        for hbb in y.hbbs() {
            out.push(Detection {
                class_id: hbb.id().unwrap_or(0),
                label: hbb.name().unwrap_or("").to_string(),
                bbox: BBox {
                    x1: hbb.xmin(),
                    y1: hbb.ymin(),
                    x2: hbb.xmax(),
                    y2: hbb.ymax(),
                },
                confidence: hbb.confidence().unwrap_or(0.0),
            });
        }
        Ok(out)
    }
}
```

> If Step 1 showed `with_version` / `with_scale` are absent or differently named in 0.1.11, drop or rename those two optional calls — they are hints only; the model file plus `yolo_detect()` is sufficient for a standard export.

Update `crates/nvr-detect/src/lib.rs` to add:

```rust
pub mod usls_backend;
pub use usls_backend::UslsDetector;
```

(add `pub mod usls_backend;` with the other `pub mod`s and `pub use usls_backend::UslsDetector;` with the other re-exports.)

- [ ] **Step 5: Write the `detect-compare` example**

Create `crates/nvr-detect/examples/detect-compare.rs`:

```rust
//! Offline comparison: run one image through every model in a manifest and
//! print a table (box count, mean confidence, inference time).
//!
//!   cargo run -p nvr-detect --example detect-compare -- \
//!     --image path/to.jpg --models third_party/detect-models/models.json \
//!     --models-dir third_party/detect-models

use std::path::{Path, PathBuf};

use clap::Parser;
use nvr_detect::{Detector, DetectorConfig, DetectorSet, UslsDetector};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    image: PathBuf,
    #[arg(long)]
    models: PathBuf,
    #[arg(long, default_value = "third_party/detect-models")]
    models_dir: PathBuf,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();

    let manifest = std::fs::read_to_string(&args.models)?;
    let cfgs: Vec<DetectorConfig> = serde_json::from_str(&manifest)?;

    let mut detectors: Vec<Box<dyn Detector>> = Vec::new();
    for cfg in &cfgs {
        let path = resolve(&args.models_dir, &cfg.model_file);
        match UslsDetector::new(cfg, &path) {
            Ok(d) => detectors.push(Box::new(d)),
            Err(e) => eprintln!("skip {}: {e:#}", cfg.name),
        }
    }
    let set = DetectorSet::new(detectors);

    let img = image::open(&args.image)?.to_rgb8();
    let (w, h) = img.dimensions();
    let results = set.detect_all(img.as_raw(), w, h);

    println!("{:<16} {:>6} {:>10} {:>10}", "model", "boxes", "mean_conf", "infer_ms");
    for r in &results {
        let mean = if r.detections.is_empty() {
            0.0
        } else {
            r.detections.iter().map(|d| d.confidence).sum::<f32>() / r.detections.len() as f32
        };
        match &r.error {
            Some(e) => println!("{:<16} ERROR {e}", r.name),
            None => println!(
                "{:<16} {:>6} {:>10.3} {:>10.1}",
                r.name,
                r.detections.len(),
                mean,
                r.infer_ms
            ),
        }
    }
    Ok(())
}

fn resolve(dir: &Path, file: &str) -> PathBuf {
    let p = Path::new(file);
    if p.is_absolute() { p.to_path_buf() } else { dir.join(p) }
}
```

- [ ] **Step 6: Build; run the live test if a model is available**

Run: `cargo build -p nvr-detect --examples` — Expected: PASS (usls + ONNX Runtime compile on first build; this is the heavy step).
Run (only if a real `yolov8n.onnx` + test image are present): the command in the `tests/live.rs` header. Expected: prints detections and PASSES.
If no model is available in the environment, note it in the task report — the ignored test is a documented manual gate, not a CI blocker.

- [ ] **Step 7: Commit**

```bash
git add crates/nvr-detect
git commit -m "feat(nvr-detect): usls YOLO backend + offline detect-compare example"
```

---

### Task 4: `ffmpeg-bus` / `media-pipe-core` `subscribe_video`

**Files:**
- Modify: `crates/ffmpeg-bus/src/frame.rs` (add `RawVideoFrame::as_video`)
- Modify: `crates/ffmpeg-bus/src/bus.rs` (`BusCommand::SubscribeVideo`, handler arm, `subscribe_video_internal`, `Bus::subscribe_video`)
- Modify: `crates/media-pipe-core/src/pipe.rs` (`Pipe::subscribe_video`)
- Test: `crates/ffmpeg-bus/src/frame_test.rs` (extend — assert `as_video` returns the inner frame)

**Interfaces:**
- Consumes: existing `subscribe_audio_internal` (bus.rs:859), `start_decoder_task` (bus.rs:1106), `AvStream::is_video()`, `RawFrameReceiver`.
- Produces:
  - `RawVideoFrame::as_video(&self) -> &ffmpeg_next::frame::Video`
  - `Bus::subscribe_video(&self) -> anyhow::Result<crate::frame::RawFrameReceiver>`
  - `Pipe::subscribe_video(&self) -> anyhow::Result<ffmpeg_bus::frame::RawFrameReceiver>`

- [ ] **Step 1: Add the `as_video` accessor (needed by nvr's converter for all planes)**

In `crates/ffmpeg-bus/src/frame.rs`, inside `impl RawVideoFrame`, add next to `data`:

```rust
    /// Borrow the inner decoded frame (all planes) — needed to feed a scaler.
    /// `data()` only exposes plane 0.
    pub fn as_video(&self) -> &ffmpeg_next::frame::Video {
        &self.frame
    }
```

- [ ] **Step 2: Write the failing test for `as_video`**

In `crates/ffmpeg-bus/src/frame_test.rs`, add:

```rust
#[test]
fn raw_video_frame_exposes_inner_via_as_video() {
    use ffmpeg_next::frame::Video;
    let src = Video::new(ffmpeg_next::format::Pixel::RGB24, 4, 2);
    let rvf = super::RawVideoFrame::from(src);
    let inner = rvf.as_video();
    assert_eq!(inner.width(), 4);
    assert_eq!(inner.height(), 2);
    assert_eq!(inner.format(), ffmpeg_next::format::Pixel::RGB24);
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p ffmpeg-bus --lib frame_test::raw_video_frame_exposes_inner_via_as_video`
Expected: FAIL to compile (`as_video` not found) — unless you did Step 1 first, in which case it PASSES immediately; that is acceptable for this accessor. Proceed either way.

- [ ] **Step 4: Add the `SubscribeVideo` command + internal + public method**

In `crates/ffmpeg-bus/src/bus.rs`, add a variant to `enum BusCommand` (after `SubscribeAudio`, ~line 1289):

```rust
    /// Subscribe to the pipe's decoded video broadcast (ensures the video
    /// decoder task is running). Receiver yields `RawFrame::Video`.
    SubscribeVideo {
        result: tokio::sync::oneshot::Sender<anyhow::Result<crate::frame::RawFrameReceiver>>,
    },
```

Add a handler arm next to the `SubscribeAudio` arm (~line 214):

```rust
            BusCommand::SubscribeVideo { result } => {
                let r = Self::subscribe_video_internal(state).await;
                let _ = result.send(r);
            }
```

Add `subscribe_video_internal` next to `subscribe_audio_internal` (~line 879), mirroring it with `is_video()`:

```rust
    /// Ensure the input + video decoder are running and return a subscription to
    /// the decoded-video broadcast. Mirrors `subscribe_audio_internal`.
    async fn subscribe_video_internal(
        state: &mut BusState,
    ) -> anyhow::Result<crate::frame::RawFrameReceiver> {
        if state.input_task.is_none() && state.input_config.is_some() {
            Self::prepare_input_task(state).await?;
        }
        let video_index = state
            .input_streams
            .iter()
            .find(|s| s.is_video())
            .ok_or_else(|| anyhow::anyhow!("pipe has no video stream"))?
            .index();
        Self::start_decoder_task(state, video_index, false).await?;
        let receiver = state
            .decoder_tasks
            .get(&video_index)
            .ok_or_else(|| anyhow::anyhow!("video decoder task not found after start"))?
            .subscribe();
        Self::start_input_task(state).await?;
        Ok(receiver)
    }
```

Add the public method next to `Bus::subscribe_audio` (~line 1221):

```rust
    /// Subscribe to this pipe's decoded-video broadcast, starting the video
    /// decoder if needed. The receiver yields `RawFrameCmd` (filter `Video`).
    pub async fn subscribe_video(&self) -> anyhow::Result<crate::frame::RawFrameReceiver> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::SubscribeVideo { result: tx })
            .await?;
        rx.await?
    }
```

- [ ] **Step 5: Forward from `media-pipe-core::Pipe`**

In `crates/media-pipe-core/src/pipe.rs`, add after `subscribe_audio` (line 50):

```rust
    /// Subscribe to this pipe's decoded-video broadcast (for detection). Errors
    /// if the pipe is not currently started.
    pub async fn subscribe_video(&self) -> anyhow::Result<ffmpeg_bus::frame::RawFrameReceiver> {
        let bus = self
            .bus
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("pipe not started"))?;
        bus.subscribe_video().await
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p ffmpeg-bus --lib frame_test`
Expected: PASS.
Run: `cargo build -p ffmpeg-bus -p media-pipe-core`
Expected: PASS. Run `cargo fmt`.

- [ ] **Step 7: Commit**

```bash
git add crates/ffmpeg-bus/src crates/media-pipe-core/src
git commit -m "feat(ffmpeg-bus): subscribe_video + RawVideoFrame::as_video"
```

---

### Task 5: `nvr/src/detect/convert.rs` — decoded frame → RGB24 bytes

**Files:**
- Create: `nvr/src/detect/mod.rs` (module declaration only, for now)
- Create: `nvr/src/detect/convert.rs`
- Test: `nvr/src/detect/convert_test.rs`
- Modify: `nvr/src/main.rs` (add `mod detect;`)
- Modify: `nvr/Cargo.toml` (add `nvr-detect` dependency)

**Interfaces:**
- Consumes: `ffmpeg_bus::frame::RawVideoFrame` (`as_video`, `width`, `height`, `format`), `ffmpeg_bus::scaler::Scaler`.
- Produces: `nvr::detect::convert::to_rgb(frame: &RawVideoFrame) -> anyhow::Result<(Vec<u8>, u32, u32)>` returning tightly-packed RGB24 bytes (`len == w*h*3`) plus dimensions.

- [ ] **Step 1: Add deps and module declarations**

In `nvr/Cargo.toml` `[dependencies]`, add:

```toml
nvr-detect = { path = "../crates/nvr-detect" }
```

In `nvr/src/main.rs`, add near the other `mod` lines (e.g. next to `mod asr;`):

```rust
mod detect;
```

Create `nvr/src/detect/mod.rs`:

```rust
//! Real-time object detection for live pipes: taps decoded video, samples,
//! fans out to N models, and serves the latest per-frame comparison over REST.

pub mod convert;
```

- [ ] **Step 2: Write the failing test**

Create `nvr/src/detect/convert_test.rs`:

```rust
use super::*;
use ffmpeg_bus::frame::RawVideoFrame;

#[test]
fn converts_yuv420p_frame_to_packed_rgb24() {
    // A 4x2 YUV420P frame (planes are allocated/zeroed by ffmpeg).
    let src = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::YUV420P, 4, 2);
    let frame = RawVideoFrame::from(src);

    let (rgb, w, h) = to_rgb(&frame).expect("convert");
    assert_eq!(w, 4);
    assert_eq!(h, 2);
    // Tightly packed RGB24: exactly w*h*3 bytes, no row padding.
    assert_eq!(rgb.len(), (4 * 2 * 3) as usize);
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p nvr detect::convert_test`
Expected: FAIL to compile (`to_rgb` not found).

- [ ] **Step 4: Implement `convert.rs`**

Create `nvr/src/detect/convert.rs`:

```rust
//! Convert a decoded video frame (any pixel format, e.g. YUV420P) into tightly-
//! packed RGB24 bytes for a detector. Reuses the ffmpeg-bus `Scaler`.

use ffmpeg_bus::frame::RawVideoFrame;
use ffmpeg_bus::scaler::Scaler;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::software::scaling::Context;
use ffmpeg_next::software::scaling::flag::Flags;

/// Returns `(rgb24_bytes, width, height)` with `rgb24_bytes.len() == w*h*3`
/// (row padding from the scaler's stride is removed).
pub fn to_rgb(frame: &RawVideoFrame) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let w = frame.width();
    let h = frame.height();
    if w == 0 || h == 0 {
        anyhow::bail!("zero-sized frame");
    }
    let src = frame.as_video();

    // Same Context::get arg order + Flags path the encoder uses (encoder.rs:531).
    let ctx = Context::get(src.format(), w, h, Pixel::RGB24, w, h, Flags::empty())?;
    let mut scaler = Scaler::new(ctx);

    // `Video::empty()` — the scaler allocates the destination (encoder idiom).
    let mut dst = ffmpeg_next::frame::Video::empty();
    scaler.run(src, &mut dst)?;

    // RGB24 has a single plane; stride may exceed w*3, so copy row by row.
    let stride = dst.stride(0);
    let row_bytes = (w as usize) * 3;
    let data = dst.data(0);
    let mut out = Vec::with_capacity(row_bytes * h as usize);
    for row in 0..h as usize {
        let start = row * stride;
        out.extend_from_slice(&data[start..start + row_bytes]);
    }
    Ok((out, w, h))
}

#[cfg(test)]
#[path = "convert_test.rs"]
mod convert_test;
```

Ensure `nvr/src/detect/mod.rs` ends with the test hook wiring already present via `pub mod convert;` (the `#[path]` mod is inside `convert.rs` itself, so nothing else is needed).

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nvr detect::convert_test`
Expected: PASS. Run `cargo fmt`.

- [ ] **Step 6: Commit**

```bash
git add nvr/Cargo.toml nvr/src/main.rs nvr/src/detect
git commit -m "feat(nvr/detect): decoded-frame to RGB24 conversion"
```

---

### Task 6: `nvr/src/detect` hub + model config + `FrameResult`

**Files:**
- Create: `nvr/src/detect/result.rs` (`FrameResult`)
- Create: `nvr/src/detect/hub.rs` (`DetectHub`)
- Modify: `nvr/src/detect/mod.rs` (declare modules; `model_config()`)
- Test: `nvr/src/detect/hub_test.rs`

**Interfaces:**
- Consumes: `nvr_detect::{Detector, DetectorConfig, ModelResult, UslsDetector}`, `tokio_util::sync::CancellationToken`.
- Produces:
  - `FrameResult { ts: i64, frame_w: u32, frame_h: u32, models: Vec<ModelResult> }` (serde `Serialize`, `Clone`).
  - `DetectHub` with: `init(configs: Vec<DetectorConfig>, models_dir: PathBuf, sample_interval_ms: u64)`, `get() -> Option<&'static DetectHub>`, `async fn detectors(&self) -> anyhow::Result<Vec<Arc<dyn Detector>>>` (lazy build all), `fn detectors_named(&self, all: &[Arc<dyn Detector>], names: &Option<Vec<String>>) -> Vec<Arc<dyn Detector>>`, `fn sample_interval_ms(&self) -> u64`, `register/unregister/is_running`, `store(pipe, FrameResult)`, `latest(pipe) -> Option<FrameResult>`, `fn config_names(&self) -> Vec<String>`.
  - `nvr::detect::model_config() -> (Vec<DetectorConfig>, PathBuf)`.

- [ ] **Step 1: Write the failing test**

Create `nvr/src/detect/hub_test.rs`:

```rust
use super::hub::DetectHub;
use super::result::FrameResult;
use nvr_detect::ModelResult;

#[test]
fn store_and_latest_roundtrip_and_register_is_idempotent() {
    // A fresh, un-init'd hub instance for isolated testing.
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::from("."), 500);

    assert!(hub.latest("cam1").is_none());
    let fr = FrameResult {
        ts: 42,
        frame_w: 1920,
        frame_h: 1080,
        models: vec![ModelResult {
            name: "m".into(),
            infer_ms: 1.0,
            detections: vec![],
            error: None,
        }],
    };
    hub.store("cam1", fr.clone());
    let got = hub.latest("cam1").expect("stored");
    assert_eq!(got.ts, 42);
    assert_eq!(got.models.len(), 1);

    let tok = tokio_util::sync::CancellationToken::new();
    assert!(hub.register("cam1", tok.clone()));
    assert!(!hub.register("cam1", tok.clone())); // already running
    assert!(hub.is_running("cam1"));
    assert!(hub.unregister("cam1"));
    assert!(!hub.is_running("cam1"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nvr detect::hub_test`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `result.rs`, `hub.rs`, and `model_config`**

Create `nvr/src/detect/result.rs`:

```rust
//! The per-frame, multi-model result served over the API.

use nvr_detect::ModelResult;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct FrameResult {
    /// Unix seconds when this frame was processed.
    pub ts: i64,
    pub frame_w: u32,
    pub frame_h: u32,
    pub models: Vec<ModelResult>,
}
```

Create `nvr/src/detect/hub.rs`:

```rust
//! Process-global detection coordination: configured models, lazily-built
//! shared detectors, the registry of running per-pipe taps, and the latest
//! per-pipe result.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use nvr_detect::{Detector, DetectorConfig, UslsDetector};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use super::result::FrameResult;

static HUB: OnceLock<DetectHub> = OnceLock::new();

pub struct DetectHub {
    configs: Vec<DetectorConfig>,
    models_dir: PathBuf,
    sample_interval_ms: u64,
    detectors: AsyncMutex<Option<Vec<Arc<dyn Detector>>>>,
    running: Mutex<HashMap<String, CancellationToken>>,
    latest: Mutex<HashMap<String, FrameResult>>,
}

impl DetectHub {
    pub fn init(configs: Vec<DetectorConfig>, models_dir: PathBuf, sample_interval_ms: u64) {
        HUB.set(Self::new_for_test(configs, models_dir, sample_interval_ms))
            .ok()
            .expect("DetectHub::init called twice");
    }

    /// Construct a hub without installing it globally (for tests).
    pub fn new_for_test(
        configs: Vec<DetectorConfig>,
        models_dir: PathBuf,
        sample_interval_ms: u64,
    ) -> Self {
        Self {
            configs,
            models_dir,
            sample_interval_ms,
            detectors: AsyncMutex::new(None),
            running: Mutex::new(HashMap::new()),
            latest: Mutex::new(HashMap::new()),
        }
    }

    pub fn get() -> Option<&'static DetectHub> {
        HUB.get()
    }

    pub fn sample_interval_ms(&self) -> u64 {
        self.sample_interval_ms
    }

    pub fn config_names(&self) -> Vec<String> {
        self.configs.iter().map(|c| c.name.clone()).collect()
    }

    /// Build (or return cached) all configured detectors. Heavy on first call
    /// (loads every ONNX model); done on a blocking thread.
    pub async fn detectors(&self) -> anyhow::Result<Vec<Arc<dyn Detector>>> {
        let mut guard = self.detectors.lock().await;
        if let Some(d) = guard.as_ref() {
            return Ok(d.clone());
        }
        if self.configs.is_empty() {
            anyhow::bail!("no models configured (missing models.json in DETECT_MODELS_DIR?)");
        }
        let configs = self.configs.clone();
        let dir = self.models_dir.clone();
        let built = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Arc<dyn Detector>>> {
            let mut out: Vec<Arc<dyn Detector>> = Vec::new();
            for cfg in &configs {
                let path = if std::path::Path::new(&cfg.model_file).is_absolute() {
                    PathBuf::from(&cfg.model_file)
                } else {
                    dir.join(&cfg.model_file)
                };
                let det = UslsDetector::new(cfg, &path)?;
                out.push(Arc::new(det));
            }
            Ok(out)
        })
        .await??;
        *guard = Some(built.clone());
        Ok(built)
    }

    /// Filter the shared detector list to a requested subset (by name). `None`
    /// or empty = all.
    pub fn detectors_named(
        &self,
        all: &[Arc<dyn Detector>],
        names: &Option<Vec<String>>,
    ) -> Vec<Arc<dyn Detector>> {
        match names {
            Some(want) if !want.is_empty() => all
                .iter()
                .filter(|d| want.iter().any(|n| n == d.name()))
                .cloned()
                .collect(),
            _ => all.to_vec(),
        }
    }

    pub fn register(&self, pipe: &str, cancel: CancellationToken) -> bool {
        let mut r = self.running.lock().unwrap();
        if r.contains_key(pipe) {
            return false;
        }
        r.insert(pipe.to_string(), cancel);
        true
    }

    pub fn unregister(&self, pipe: &str) -> bool {
        let mut r = self.running.lock().unwrap();
        if let Some(tok) = r.remove(pipe) {
            tok.cancel();
            true
        } else {
            false
        }
    }

    pub fn is_running(&self, pipe: &str) -> bool {
        self.running.lock().unwrap().contains_key(pipe)
    }

    pub fn store(&self, pipe: &str, result: FrameResult) {
        self.latest
            .lock()
            .unwrap()
            .insert(pipe.to_string(), result);
    }

    pub fn latest(&self, pipe: &str) -> Option<FrameResult> {
        self.latest.lock().unwrap().get(pipe).cloned()
    }
}
```

Update `nvr/src/detect/mod.rs`:

```rust
//! Real-time object detection for live pipes: taps decoded video, samples,
//! fans out to N models, and serves the latest per-frame comparison over REST.

pub mod convert;
pub mod hub;
pub mod result;

use std::path::PathBuf;

use nvr_detect::DetectorConfig;

/// Resolve the configured models from `DETECT_MODELS_DIR/models.json`. Returns
/// an empty config list (not an error) when the manifest is absent, so the app
/// still boots; `start` then reports "no models configured".
pub fn model_config() -> (Vec<DetectorConfig>, PathBuf) {
    let dir = std::env::var("DETECT_MODELS_DIR")
        .unwrap_or_else(|_| "third_party/detect-models".to_string());
    let dir = PathBuf::from(dir);
    let manifest = dir.join("models.json");
    let configs = match std::fs::read_to_string(&manifest) {
        Ok(s) => serde_json::from_str::<Vec<DetectorConfig>>(&s).unwrap_or_else(|e| {
            log::warn!("detect: bad {}: {e:#}", manifest.display());
            vec![]
        }),
        Err(_) => {
            log::info!("detect: no manifest at {}", manifest.display());
            vec![]
        }
    };
    (configs, dir)
}

#[cfg(test)]
#[path = "hub_test.rs"]
mod hub_test;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nvr detect::hub_test`
Expected: PASS. Run `cargo fmt`.

- [ ] **Step 5: Commit**

```bash
git add nvr/src/detect
git commit -m "feat(nvr/detect): DetectHub, FrameResult, model_config loader"
```

---

### Task 7: `nvr/src/detect/tap.rs` — sampling + concurrent fan-out

**Files:**
- Create: `nvr/src/detect/tap.rs`
- Modify: `nvr/src/detect/mod.rs` (`pub mod tap;`)
- Test: `nvr/src/detect/tap_test.rs`

**Interfaces:**
- Consumes: `ffmpeg_bus::frame::{RawFrame, RawFrameCmd, RawFrameReceiver}`, `nvr_detect::{Detector, ModelResult}`, `super::convert::to_rgb`, `super::hub::DetectHub`, `super::result::FrameResult`.
- Produces: `nvr::detect::tap::run(pipe: String, detectors: Vec<Arc<dyn Detector>>, video: RawFrameReceiver, sample_interval_ms: u64, hub: &'static DetectHub, cancel: CancellationToken)` (async) — samples, converts, fans out, stores each `FrameResult`; and a pure `fanout(detectors, rgb, w, h) -> Vec<ModelResult>` helper made testable.

- [ ] **Step 1: Write the failing test**

Create `nvr/src/detect/tap_test.rs`:

```rust
use super::tap;
use nvr_detect::{BBox, Detection, Detector};
use std::sync::Arc;

struct Fake(String);
impl Detector for Fake {
    fn name(&self) -> &str {
        &self.0
    }
    fn detect(&self, _rgb: &[u8], _w: u32, _h: u32) -> anyhow::Result<Vec<Detection>> {
        Ok(vec![Detection {
            class_id: 0,
            label: "person".into(),
            bbox: BBox { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 },
            confidence: 0.9,
        }])
    }
}

#[tokio::test]
async fn fanout_runs_every_detector_concurrently() {
    let dets: Vec<Arc<dyn Detector>> =
        vec![Arc::new(Fake("a".into())), Arc::new(Fake("b".into()))];
    let rgb = Arc::new(vec![0u8; 3]);
    let results = tap::fanout(&dets, rgb, 1, 1).await;

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"a") && names.contains(&"b"));
    assert!(results.iter().all(|r| r.detections.len() == 1));
    assert!(results.iter().all(|r| r.error.is_none()));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nvr detect::tap_test`
Expected: FAIL to compile (`tap::fanout` not found).

- [ ] **Step 3: Implement `tap.rs`**

Create `nvr/src/detect/tap.rs`:

```rust
//! Per-pipe detection tap: decoded video -> sample -> RGB -> N models -> store.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ffmpeg_bus::frame::{RawFrame, RawFrameCmd, RawFrameReceiver};
use nvr_detect::{Detector, ModelResult};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use super::convert::to_rgb;
use super::hub::DetectHub;
use super::result::FrameResult;

/// Run every detector on the same RGB image concurrently (each on a blocking
/// thread, since ONNX inference is CPU-bound), preserving order in the output.
pub async fn fanout(
    detectors: &[Arc<dyn Detector>],
    rgb: Arc<Vec<u8>>,
    w: u32,
    h: u32,
) -> Vec<ModelResult> {
    let mut handles = Vec::with_capacity(detectors.len());
    for det in detectors {
        let det = det.clone();
        let rgb = rgb.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            let res = det.detect(&rgb, w, h);
            let infer_ms = start.elapsed().as_secs_f64() * 1000.0;
            let name = det.name().to_string();
            match res {
                Ok(detections) => ModelResult { name, infer_ms, detections, error: None },
                Err(e) => ModelResult {
                    name,
                    infer_ms,
                    detections: vec![],
                    error: Some(format!("{e:#}")),
                },
            }
        }));
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        match h.await {
            Ok(r) => out.push(r),
            Err(e) => log::warn!("detect: fanout task join error: {e}"),
        }
    }
    out
}

/// Drive one pipe's detection until `cancel` fires or the video broadcast ends.
pub async fn run(
    pipe: String,
    detectors: Vec<Arc<dyn Detector>>,
    mut video: RawFrameReceiver,
    sample_interval_ms: u64,
    hub: &'static DetectHub,
    cancel: CancellationToken,
) {
    let interval = Duration::from_millis(sample_interval_ms);
    let mut last: Option<Instant> = None;

    loop {
        let cmd = tokio::select! {
            _ = cancel.cancelled() => break,
            r = video.recv() => r,
        };
        match cmd {
            Ok(RawFrameCmd::Data(RawFrame::Video(vf))) => {
                let now = Instant::now();
                if let Some(l) = last {
                    if now.duration_since(l) < interval {
                        continue; // drop frames faster than the sample rate
                    }
                }
                last = Some(now);

                let (rgb, w, h) = match to_rgb(&vf) {
                    Ok(t) => t,
                    Err(e) => {
                        log::debug!("detect[{pipe}]: convert error: {e:#}");
                        continue;
                    }
                };
                let models = fanout(&detectors, Arc::new(rgb), w, h).await;
                hub.store(
                    &pipe,
                    FrameResult { ts: chrono::Utc::now().timestamp(), frame_w: w, frame_h: h, models },
                );
            }
            Ok(RawFrameCmd::Data(RawFrame::Audio(_))) => {}
            Ok(RawFrameCmd::EOF) => break,
            Err(RecvError::Lagged(n)) => {
                log::debug!("detect[{pipe}]: dropped {n} frames (lag)");
            }
            Err(RecvError::Closed) => break,
        }
    }
    log::info!("detect[{pipe}]: tap stopped");
}

#[cfg(test)]
#[path = "tap_test.rs"]
mod tap_test;
```

Add `pub mod tap;` to `nvr/src/detect/mod.rs` (with the other `pub mod`s).

Confirm `chrono` is a dependency of the `nvr` crate (it is a workspace dep). If `cargo build` reports `chrono` unresolved, add `chrono = { workspace = true }` to `nvr/Cargo.toml`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nvr detect::tap_test`
Expected: PASS. Run `cargo fmt`.

- [ ] **Step 5: Commit**

```bash
git add nvr/src/detect nvr/Cargo.toml
git commit -m "feat(nvr/detect): sampling tap with concurrent model fan-out"
```

---

### Task 8: REST API, wiring, docs, and final verification

**Files:**
- Create: `nvr/src/detect/api.rs`
- Modify: `nvr/src/detect/mod.rs` (`pub mod api;`)
- Modify: `nvr/src/api.rs` (mount `/detect` router + `DetectHub::init`)
- Create: `crates/nvr-detect/README.md`
- Create: `third_party/detect-models/models.json.example`
- Test: `nvr/src/detect/api_test.rs`

**Interfaces:**
- Consumes: `super::hub::DetectHub`, `crate::manager::get_pipe`, `nvr_detect::Detector`, `super::tap`.
- Produces: `nvr::detect::api::detect_router() -> axum::Router` with `POST /{pipe}/start`, `POST /{pipe}/stop`, `GET /{pipe}/latest`, `GET /models`.

- [ ] **Step 1: Write the failing test**

Create `nvr/src/detect/api_test.rs`:

```rust
use super::api::StartBody;

#[test]
fn start_body_defaults_models_to_none() {
    // Empty body → run all configured models.
    let b: StartBody = serde_json::from_str("{}").unwrap();
    assert!(b.models.is_none());

    let b: StartBody = serde_json::from_str(r#"{"models":["yolov8n"]}"#).unwrap();
    assert_eq!(b.models.unwrap(), vec!["yolov8n".to_string()]);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nvr detect::api_test`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `api.rs`**

Create `nvr/src/detect/api.rs`:

```rust
//! Detection control + read endpoints. Opt-in start/stop per pipe; GET latest
//! per-frame multi-model result. GET/POST only; session auth is applied by the
//! parent `/api` router.

use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::hub::DetectHub;

#[derive(Deserialize, Default)]
pub struct StartBody {
    /// Subset of configured model names to run. Absent/empty = all.
    #[serde(default)]
    pub models: Option<Vec<String>>,
}

pub fn detect_router() -> Router {
    Router::new()
        .route("/{pipe}/start", post(start))
        .route("/{pipe}/stop", post(stop))
        .route("/{pipe}/latest", get(latest))
        .route("/models", get(models))
}

async fn models() -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    Json(hub.config_names()).into_response()
}

async fn latest(Path(pipe): Path<String>) -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    match hub.latest(&pipe) {
        Some(fr) => Json(fr).into_response(),
        None => (StatusCode::NOT_FOUND, "no result yet").into_response(),
    }
}

async fn stop(Path(pipe): Path<String>) -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    if hub.unregister(&pipe) {
        (StatusCode::OK, "stopped").into_response()
    } else {
        (StatusCode::OK, "not running").into_response()
    }
}

async fn start(Path(pipe): Path<String>, body: Option<Json<StartBody>>) -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    if hub.is_running(&pipe) {
        return (StatusCode::OK, "already running").into_response();
    }
    let want = body.and_then(|Json(b)| b.models);

    let Some(handle) = crate::manager::get_pipe(&pipe).await else {
        return (StatusCode::NOT_FOUND, "pipe not found").into_response();
    };
    let video = match handle.subscribe_video().await {
        Ok(rx) => rx,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("no video: {e:#}")).into_response(),
    };

    let all = match hub.detectors().await {
        Ok(d) => d,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("model load failed: {e:#}"))
                .into_response();
        }
    };
    let detectors = hub.detectors_named(&all, &want);
    if detectors.is_empty() {
        return (StatusCode::BAD_REQUEST, "no matching models").into_response();
    }

    let cancel = CancellationToken::new();
    if !hub.register(&pipe, cancel.clone()) {
        return (StatusCode::OK, "already running").into_response();
    }
    let interval = hub.sample_interval_ms();
    tokio::spawn(super::tap::run(pipe, detectors, video, interval, hub, cancel));
    (StatusCode::OK, "started").into_response()
}

#[cfg(test)]
#[path = "api_test.rs"]
mod api_test;
```

Add `pub mod api;` to `nvr/src/detect/mod.rs`.

- [ ] **Step 4: Mount the router and init the hub in `nvr/src/api.rs`**

In `nvr/src/api.rs`, add the nest next to the `asr`/`onvif` nests (after line 19):

```rust
            .nest("/detect", crate::detect::api::detect_router())
```

And init the hub next to `AsrHub::init` (after line 37):

```rust
        {
            let (configs, dir) = crate::detect::model_config();
            crate::detect::hub::DetectHub::init(configs, dir, 500);
        }
```

- [ ] **Step 5: Run the api test and build the whole workspace**

Run: `cargo test -p nvr detect::api_test`
Expected: PASS.
Run: `cargo build -p nvr` then `cargo test -p nvr --lib detect::`
Expected: PASS. Run `cargo fmt`.

- [ ] **Step 6: Write the docs and the manifest example**

Create `crates/nvr-detect/README.md`:

````markdown
# nvr-detect — multi-model object detection

A backend-agnostic detection component: a `Detector` trait, unified
`Detection` output (label, bbox in pixels, confidence), and a `usls`
(ONNX Runtime) YOLO backend. `DetectorSet` runs one image through N models
and returns each model's timed result.

## Offline comparison

Put ONNX weights + a `models.json` under `third_party/detect-models/`:

```json
[
  { "name": "yolov8n", "model_file": "yolov8n.onnx", "version": 8.0 },
  { "name": "yolo11s", "model_file": "yolo11s.onnx", "version": 11.0 }
]
```

Weights are not committed. Export from Ultralytics, e.g.
`yolo export model=yolov8n.pt format=onnx`, and drop the `.onnx` in that dir.

```bash
cargo run -p nvr-detect --example detect-compare -- \
  --image some.jpg --models third_party/detect-models/models.json \
  --models-dir third_party/detect-models
```

## Real-time (in nvr)

`nvr` taps a running pipe's decoded video, samples (~2fps), fans each frame out
to the configured models, and serves the latest result. The API (port 18080) is
session-auth guarded, so log in first (`admin`/`admin`) and pass the token.

```bash
TOKEN=$(curl -s -X POST localhost:18080/api/user/login \
  -H 'content-type: application/json' -d '{"username":"admin","password":"admin"}' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["token"])')

curl -s "localhost:18080/api/detect/models?token=$TOKEN"
curl -s -X POST "localhost:18080/api/detect/<pipe>/start?token=$TOKEN" \
  -H 'content-type: application/json' -d '{"models":["yolov8n","yolo11s"]}'
curl -s "localhost:18080/api/detect/<pipe>/latest?token=$TOKEN"
curl -s -X POST "localhost:18080/api/detect/<pipe>/stop?token=$TOKEN"
```

`GET /latest` returns `{ ts, frame_w, frame_h, models: [{ name, infer_ms,
detections: [{ class_id, label, bbox:{x1,y1,x2,y2}, confidence }], error }] }`.
Coordinates are original-frame pixels; scale by `frame_w`/`frame_h`.

Set `DETECT_MODELS_DIR` to point elsewhere (default `third_party/detect-models`).
````

Create `third_party/detect-models/models.json.example`:

```json
[
  { "name": "yolov8n", "model_file": "yolov8n.onnx", "version": 8.0, "conf": 0.25 },
  { "name": "yolo11s", "model_file": "yolo11s.onnx", "version": 11.0, "conf": 0.25 }
]
```

- [ ] **Step 7: Full workspace verify + manual E2E note**

Run: `cargo fmt --all && cargo build --workspace && cargo test -p nvr-detect -p nvr -p ffmpeg-bus -p media-pipe-core --lib`
Expected: PASS, no warnings in the new crates.

Manual E2E (document the result in the task report; requires a model file):
1. Place `yolov8n.onnx` + `models.json` in `third_party/detect-models/`.
2. Start `dummy-rtsp-camera`, start `nvr`.
3. Add an RTSP device pointing at the dummy; note its pipe id.
4. `POST /api/detect/<pipe>/start` with `{"models":["yolov8n"]}`.
5. `GET /api/detect/<pipe>/latest` → a `FrameResult` with detections.
6. `POST /api/detect/<pipe>/stop`.

- [ ] **Step 8: Commit**

```bash
git add nvr/src/detect nvr/src/api.rs crates/nvr-detect/README.md third_party/detect-models/models.json.example
git commit -m "feat(nvr/detect): REST API, router+hub wiring, docs"
```

---

## Notes for the executor

- **usls compile time.** The first `cargo build` after Task 3 adds usls + ONNX
  Runtime and will be slow (native lib download/build). Budget for it; a
  timeout is not a failure.
- **usls API drift.** Task 3 Step 1 is a hard gate: verify the builder/output
  names against the installed 0.1.11 before writing `usls_backend.rs`. If
  `with_version`/`with_scale` don't exist, drop them (model file + `yolo_detect`
  suffices for a standard export). `input_size` and `iou` are manifest hints —
  usls derives real input dims from the model and runs NMS internally; wire them
  via `with_model_ixx`/`with_iou` only if Step 1 confirms those setters exist.
  The `DetectorConfig` fields stay regardless (they are the manifest contract and
  are consumed by serde, so they are not dead code).
- **No model in the environment.** The ignored live test and the manual E2E
  need a real `.onnx`. If none is available, everything else (types, config,
  set, subscribe_video, convert, hub, tap fan-out, api) is fully covered by the
  non-ignored tests; report the live/E2E steps as "not run — no model".
- **Stale rust-analyzer diagnostics** in this repo are common mid-edit; trust
  `cargo build`/`cargo test`, not the diagnostics panel.
