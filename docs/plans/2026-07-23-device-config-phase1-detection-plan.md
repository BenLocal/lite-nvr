# Per-device Detection (Device-config Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each device persist a detection config (enable, model subset, sample interval, confidence floor) that the backend auto-starts when the device's pipe runs, edited from a new **检测** tab in the device add/edit dialog.

**Architecture:** Devices are stored as JSON in the KV table, so `DeviceInfo` gains a defaulted `config: DeviceConfig { detect: Option<DetectConfig> }` — no migration. A shared `detect::control::start_tap` factors the tap-start sequence out of the REST handler so `ensure_device_pipe` can auto-start detection from persisted config. A per-device confidence floor is applied as a cheap post-inference filter. The frontend wraps the existing device form in a PrimeVue `Tabs` and adds the 检测 tab.

**Tech Stack:** Rust (edition 2024, Axum, turso/SQLite KV, tokio); Vue 3 + TypeScript + PrimeVue (`@primevue/forms`, Tabs, MultiSelect, Slider, InputNumber, ToggleSwitch).

## Global Constraints

- Rust edition 2024; run `cargo fmt` before each backend commit.
- Tests colocate as `*_test.rs` next to source, wired via `#[cfg(test)] #[path = "..._test.rs"] mod ..._test;`.
- Detection applies to **pipe-backed devices only**; `input_type == "gb28181"` is excluded (no pipe). The 检测 tab is hidden for gb28181.
- Confidence floor is a **post-inference filter** (no per-device model rebuild). The existing manual `/detect/{pipe}/start|stop` API and the preview overlay stay working (overlay only checks `res.ok`).
- **No DB migration** — device config rides in the existing KV JSON blob; new struct fields use `#[serde(default)]` for back-compat with existing rows.
- Frontend: use PrimeVue components; all HTTP lives in `src/api/`; `npm run type-check` and `npm run lint` must pass before any frontend commit. Keep the dark control-room theme (reuse `.field` / `.field-grid` / `.field-hint`).
- API verbs are GET/POST only.
- **Backend build/test environment:** `cargo build/test -p nvr` needs the FFmpeg + ZLM shared libs on the loader path. Either run via the Makefile (`make build`, `make test`) which exports `FFMPEG_DIR`/`ZLM_DIR`/`LD_LIBRARY_PATH`, or prefix cargo directly: `LD_LIBRARY_PATH="$(pwd)/ffmpeg/lib:$(pwd)/target/debug/deps:$LD_LIBRARY_PATH" cargo test -p nvr <filter>`. (`nvr-db` tests need no special env.)
- **Colocated-test filter:** a `*_test.rs` colocated inside a submodule file registers under `<parent>::<file_stem>::<file_stem>_test::<name>`. Filter with `<parent>::<file_stem>` (e.g. `detect::tap`, `detect::control`) — `detect::tap_test` silently matches 0 tests and prints a misleading "0 filtered out". Inside the test file, `super` refers to the parent file's module (e.g. `super::apply_min_confidence`), not a `super::<file_stem>::…` path.
- **Tap registration is generation-scoped:** `DetectHub::register` returns an `Option<TapEpoch>` and `tap::run` releases its own slot via `unregister_tap(pipe, epoch)` when it ends (stream EOF / device disconnect), so a dead tap never leaves `is_running` stuck true. Keep the `epoch` argument when threading new params through `run`.
- **Expected `dead_code` warning:** a function added before its caller (wired in a later task) produces a `warning: function '…' is never used`. nvr does not deny warnings, so this is expected and not a failure (e.g. `reconcile_detection`/`stop_detection` are unused until Task 4).

## File Structure

**Backend**
- `nvr-db/src/device.rs` — add `DeviceConfig` + `DetectConfig` types and `DeviceInfo.config`.
- `nvr-db/src/device_test.rs` — **new**; serde back-compat + round-trip tests.
- `nvr/src/detect/tap.rs` — add `apply_min_confidence` helper + `min_confidence` param on `run`.
- `nvr/src/detect/tap_test.rs` — extend with the filter test.
- `nvr/src/detect/control.rs` — **new**; `StartOutcome`, `start_tap`, `stop_detection`, `reconcile_detection`, `should_auto_start`.
- `nvr/src/detect/control_test.rs` — **new**; `should_auto_start` decision tests.
- `nvr/src/detect/mod.rs` — declare `pub(crate) mod control;`.
- `nvr/src/detect/api.rs` — refactor `start` to delegate to `control::start_tap`.
- `nvr/src/handler/device.rs` — add `config` to `DevicePayload`; set it on both `DeviceInfo` literals; stop detection in `remove_device`.
- `nvr/src/init/device.rs` — call `reconcile_detection` at the three pipe-creating return points.

**Frontend**
- `nvr-dashboard/app/src/api/device.ts` — add `DeviceConfig`/`DetectConfig` types; `config?` on `DeviceItem` and `DevicePayload`.
- `nvr-dashboard/app/src/views/DeviceListView.vue` — detect refs + model loader + reset/hydrate; wrap form in `Tabs`; add 检测 `TabPanel`; assemble `config` in `onSubmit`.

---

### Task 1: Per-device config model + persistence

**Files:**
- Modify: `nvr-db/src/device.rs`
- Test: `nvr-db/src/device_test.rs` (create)
- Modify: `nvr/src/handler/device.rs`

**Interfaces:**
- Produces: `nvr_db::device::DeviceConfig { detect: Option<DetectConfig> }` (derives `Default`, `Clone`, `Serialize`, `Deserialize`); `nvr_db::device::DetectConfig { enabled: bool, models: Vec<String>, sample_every_ms: u64, min_confidence: f32 }`; `DeviceInfo.config: DeviceConfig` (`#[serde(default)]`).

- [ ] **Step 1: Write the failing test**

Create `nvr-db/src/device_test.rs`:

```rust
// device_test is a child module of `device` (this file), so `super` == device.
use super::{DetectConfig, DeviceConfig, DeviceInfo};

#[test]
fn device_without_config_deserializes_to_default() {
    // A row written before this field exists must still load, with detection off.
    let json = r#"{
        "id":"cam1","name":"Cam 1","input_type":"rtsp","input_value":"rtsp://x",
        "description":"","include_audio":false,"record":true,
        "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"
    }"#;
    let dev: DeviceInfo = serde_json::from_str(json).expect("legacy row must parse");
    assert!(dev.config.detect.is_none());
}

#[test]
fn detect_config_round_trips() {
    let cfg = DeviceConfig {
        detect: Some(DetectConfig {
            enabled: true,
            models: vec!["yolov8n".to_string()],
            sample_every_ms: 500,
            min_confidence: 0.4,
        }),
    };
    let s = serde_json::to_string(&cfg).unwrap();
    let back: DeviceConfig = serde_json::from_str(&s).unwrap();
    let d = back.detect.expect("detect present");
    assert!(d.enabled);
    assert_eq!(d.models, vec!["yolov8n".to_string()]);
    assert_eq!(d.sample_every_ms, 500);
    assert!((d.min_confidence - 0.4).abs() < 1e-6);
}
```

Wire the test module by appending to the end of `nvr-db/src/device.rs`:

```rust
#[cfg(test)]
#[path = "device_test.rs"]
mod device_test;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nvr-db device_test`
Expected: FAIL — compile error (`DeviceConfig`/`DetectConfig` unresolved, `DeviceInfo` has no field `config`).

- [ ] **Step 3: Add the types and field**

In `nvr-db/src/device.rs`, add after the `DeviceInfo` struct (keep `use serde::{Deserialize, Serialize};` — already imported):

```rust
/// Per-device configuration blob (KV JSON). Optional sections are added by
/// later device-config phases (stream / transport / recording).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect: Option<DetectConfig>,
}

/// Per-device object-detection settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectConfig {
    pub enabled: bool,
    /// Subset of configured model names to run. Empty = all configured models.
    #[serde(default)]
    pub models: Vec<String>,
    /// Sampling interval in ms. 0 = use the hub default.
    #[serde(default)]
    pub sample_every_ms: u64,
    /// Post-inference confidence floor. 0.0 = keep each model's built-in conf.
    #[serde(default)]
    pub min_confidence: f32,
}
```

Add the field to `DeviceInfo` (after `record`):

```rust
    #[serde(default)]
    pub config: DeviceConfig,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nvr-db device_test`
Expected: PASS (2 tests).

- [ ] **Step 5: Persist config through the device handler**

In `nvr/src/handler/device.rs`, extend the import that pulls in `DeviceInfo` to also bring `DeviceConfig` (find the existing `use nvr_db::device::...` / `DeviceInfo` use and add `DeviceConfig`; if `DeviceInfo` is referenced by full path, add `use nvr_db::device::DeviceConfig;`).

Add the field to `DevicePayload` (after `record`):

```rust
    #[serde(default)]
    config: DeviceConfig,
```

In `add_device`, set it on the `DeviceInfo` literal (add alongside `record`):

```rust
        config: payload.config,
```

In `update_device`, set it on that `DeviceInfo` literal too:

```rust
        config: payload.config,
```

- [ ] **Step 6: Verify the workspace builds**

Run: `cargo build -p nvr`
Expected: builds clean (no missing-field errors on the `DeviceInfo` literals).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add nvr-db/src/device.rs nvr-db/src/device_test.rs nvr/src/handler/device.rs
git commit -m "feat(device): persist per-device DeviceConfig with optional detect section"
```

---

### Task 2: Confidence-floor post-filter in the detection tap

**Files:**
- Modify: `nvr/src/detect/tap.rs`
- Test: `nvr/src/detect/tap_test.rs`
- Modify: `nvr/src/detect/api.rs` (only caller of `tap::run`, keep it compiling)

**Interfaces:**
- Produces: `pub(crate) fn apply_min_confidence(models: &mut [ModelResult], min: f32)`; `tap::run(..., min_confidence: f32)` gains a trailing `min_confidence` parameter.

- [ ] **Step 1: Write the failing test**

Append to `nvr/src/detect/tap_test.rs`:

```rust
use nvr_detect::{BBox, Detection, ModelResult};

fn det(conf: f32) -> Detection {
    Detection {
        class_id: 0,
        label: "obj".to_string(),
        bbox: BBox { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 },
        confidence: conf,
    }
}

#[test]
fn min_confidence_drops_low_scores() {
    let mut models = vec![ModelResult {
        name: "m".to_string(),
        infer_ms: 1.0,
        detections: vec![det(0.9), det(0.3), det(0.5)],
        error: None,
    }];
    super::apply_min_confidence(&mut models, 0.5);
    let confs: Vec<f32> = models[0].detections.iter().map(|d| d.confidence).collect();
    assert_eq!(confs, vec![0.9, 0.5]);
}

#[test]
fn min_confidence_zero_is_noop() {
    let mut models = vec![ModelResult {
        name: "m".to_string(),
        infer_ms: 1.0,
        detections: vec![det(0.1)],
        error: None,
    }];
    super::apply_min_confidence(&mut models, 0.0);
    assert_eq!(models[0].detections.len(), 1);
}
```

(If `tap_test.rs` does not yet exist, create it with just the above; `tap.rs` already declares `mod tap_test;` at its end.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nvr detect::tap`
Expected: FAIL — `apply_min_confidence` not found.

- [ ] **Step 3: Add the helper and thread the parameter**

In `nvr/src/detect/tap.rs`, add the helper (above `run`):

```rust
/// Drop detections whose confidence is below `min` from every model result.
/// `min <= 0.0` is a no-op (keep the model's built-in threshold).
pub(crate) fn apply_min_confidence(models: &mut [ModelResult], min: f32) {
    if min <= 0.0 {
        return;
    }
    for m in models.iter_mut() {
        m.detections.retain(|d| d.confidence >= min);
    }
}
```

Add `min_confidence: f32` as the final parameter of `run`. Note `epoch: TapEpoch`
(added by the zombie-registration fix) sits between `hub` and `cancel` — keep it:

```rust
pub async fn run(
    pipe: String,
    detectors: Vec<Arc<dyn Detector>>,
    mut video: RawFrameReceiver,
    sample_interval_ms: u64,
    hub: &'static DetectHub,
    epoch: TapEpoch,
    cancel: CancellationToken,
    min_confidence: f32,
) {
```

Apply it after `fanout`, before `hub.store` (replace the existing `let models = fanout(...)` line):

```rust
                let mut models = fanout(&detectors, Arc::new(rgb), w, h).await;
                apply_min_confidence(&mut models, min_confidence);
```

- [ ] **Step 4: Keep the sole caller compiling**

In `nvr/src/detect/api.rs`, the `tokio::spawn(super::tap::run(...))` call in `start` must pass the new arg. Update it to:

```rust
    tokio::spawn(super::tap::run(
        pipe, detectors, video, interval, hub, epoch, cancel, 0.0,
    ));
```

- [ ] **Step 5: Run tests + build to verify pass**

Run: `cargo test -p nvr detect::tap`
Expected: PASS (2 new tests).
Run: `cargo build -p nvr`
Expected: builds clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add nvr/src/detect/tap.rs nvr/src/detect/tap_test.rs nvr/src/detect/api.rs
git commit -m "feat(detect): per-tap min_confidence post-inference filter"
```

---

### Task 3: Shared start/stop control + auto-start reconcile logic

**Files:**
- Create: `nvr/src/detect/control.rs`
- Create: `nvr/src/detect/control_test.rs`
- Modify: `nvr/src/detect/mod.rs`
- Modify: `nvr/src/detect/api.rs`

**Interfaces:**
- Consumes: `DetectHub::{get, is_running, detectors, detectors_named, register, unregister, sample_interval_ms}` (`register` returns `Option<TapEpoch>` — `None` means already running); `crate::manager::get_pipe(&str) -> Option<handle>` where `handle.subscribe_video().await -> anyhow::Result<RawFrameReceiver>`; `tap::run(..., min_confidence)` from Task 2; `nvr_db::device::{DeviceInfo, DetectConfig}` from Task 1.
- Produces: `pub(crate) enum StartOutcome { Started, AlreadyRunning }`; `pub(crate) async fn start_tap(hub: &'static DetectHub, pipe: &str, want: Option<Vec<String>>, sample_interval_ms: u64, min_confidence: f32) -> anyhow::Result<StartOutcome>`; `pub(crate) fn stop_detection(pipe: &str)`; `pub(crate) async fn reconcile_detection(device: &DeviceInfo)`; `pub(crate) fn should_auto_start(detect: Option<&DetectConfig>, input_type: &str) -> bool`.

- [ ] **Step 1: Write the failing test**

Create `nvr/src/detect/control_test.rs`:

```rust
// control_test is a child module of `control` (this file), so `super` == control.
use super::should_auto_start;
use nvr_db::device::DetectConfig;

fn cfg(enabled: bool) -> DetectConfig {
    DetectConfig { enabled, models: vec![], sample_every_ms: 0, min_confidence: 0.0 }
}

#[test]
fn auto_start_when_enabled_and_pipe_backed() {
    assert!(should_auto_start(Some(&cfg(true)), "rtsp"));
}

#[test]
fn no_auto_start_when_disabled() {
    assert!(!should_auto_start(Some(&cfg(false)), "rtsp"));
}

#[test]
fn no_auto_start_when_absent() {
    assert!(!should_auto_start(None, "rtsp"));
}

#[test]
fn no_auto_start_for_gb28181() {
    assert!(!should_auto_start(Some(&cfg(true)), "gb28181"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nvr detect::control`
Expected: FAIL — module `control` does not exist.

- [ ] **Step 3: Create the control module**

Create `nvr/src/detect/control.rs`:

```rust
//! Shared detection start/stop plumbing. Both the REST `start` handler and
//! device-config auto-start go through `start_tap`, so the tap is built the
//! same way regardless of trigger.

use tokio_util::sync::CancellationToken;

use nvr_db::device::{DetectConfig, DeviceInfo};

use super::hub::DetectHub;

pub(crate) enum StartOutcome {
    Started,
    AlreadyRunning,
}

/// Whether a device should have detection auto-started: enabled config on a
/// pipe-backed device. gb28181 has no pipe, so it never qualifies.
pub(crate) fn should_auto_start(detect: Option<&DetectConfig>, input_type: &str) -> bool {
    detect.is_some_and(|d| d.enabled) && input_type != "gb28181"
}

/// Start a detection tap for `pipe`. `sample_interval_ms == 0` uses the hub
/// default. Idempotent: returns `AlreadyRunning` if a tap is already registered.
pub(crate) async fn start_tap(
    hub: &'static DetectHub,
    pipe: &str,
    want: Option<Vec<String>>,
    sample_interval_ms: u64,
    min_confidence: f32,
) -> anyhow::Result<StartOutcome> {
    if hub.is_running(pipe) {
        return Ok(StartOutcome::AlreadyRunning);
    }
    let handle = crate::manager::get_pipe(pipe)
        .await
        .ok_or_else(|| anyhow::anyhow!("pipe not found"))?;
    let video = handle
        .subscribe_video()
        .await
        .map_err(|e| anyhow::anyhow!("no video: {e:#}"))?;

    let all = hub.detectors().await?;
    let detectors = hub.detectors_named(&all, &want);
    if detectors.is_empty() {
        anyhow::bail!("no matching models");
    }

    let cancel = CancellationToken::new();
    let Some(epoch) = hub.register(pipe, cancel.clone()) else {
        return Ok(StartOutcome::AlreadyRunning);
    };
    let interval = if sample_interval_ms > 0 {
        sample_interval_ms
    } else {
        hub.sample_interval_ms()
    };
    tokio::spawn(super::tap::run(
        pipe.to_string(),
        detectors,
        video,
        interval,
        hub,
        epoch,
        cancel,
        min_confidence,
    ));
    Ok(StartOutcome::Started)
}

/// Stop a running tap for `pipe` (idempotent no-op if not running / hub down).
pub(crate) fn stop_detection(pipe: &str) {
    if let Some(hub) = DetectHub::get() {
        hub.unregister(pipe);
    }
}

/// Reconcile a device's persisted detection config against the running tap:
/// start when enabled + pipe-backed, stop otherwise.
pub(crate) async fn reconcile_detection(device: &DeviceInfo) {
    let want_on = should_auto_start(device.config.detect.as_ref(), &device.input_type);

    let Some(hub) = DetectHub::get() else {
        if want_on {
            log::warn!(
                "detect: hub not initialized; skipping auto-start for {}",
                device.id
            );
        }
        return;
    };

    if !want_on {
        hub.unregister(&device.id);
        return;
    }

    // want_on implies detect is Some(enabled).
    let cfg = device.config.detect.as_ref().unwrap();
    let want = if cfg.models.is_empty() {
        None
    } else {
        Some(cfg.models.clone())
    };
    if let Err(e) = start_tap(hub, &device.id, want, cfg.sample_every_ms, cfg.min_confidence).await {
        log::warn!("detect: auto-start failed for {}: {e:#}", device.id);
    }
}

#[cfg(test)]
#[path = "control_test.rs"]
mod control_test;
```

- [ ] **Step 4: Declare the module**

In `nvr/src/detect/mod.rs`, add alongside the other `pub mod` lines:

```rust
pub(crate) mod control;
```

- [ ] **Step 5: Refactor the REST `start` handler to delegate**

In `nvr/src/detect/api.rs`, replace the body of `async fn start(...)` (the whole function from `let Some(hub) = ...` through the final `into_response()`) with:

```rust
async fn start(Path(pipe): Path<String>, body: Option<Json<StartBody>>) -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    let want = body.and_then(|Json(b)| b.models);
    match crate::detect::control::start_tap(hub, &pipe, want, 0, 0.0).await {
        Ok(crate::detect::control::StartOutcome::Started) => {
            (StatusCode::OK, "started").into_response()
        }
        Ok(crate::detect::control::StartOutcome::AlreadyRunning) => {
            (StatusCode::OK, "already running").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}
```

Then remove now-unused imports in `api.rs` if the compiler flags them: `CancellationToken` (was only used by the old body). Leave `StartBody`, `Json`, `Path`, `StatusCode`, `get`, `post`, `DetectHub` as they are still used. Note: error cases (`pipe not found` / `no video` / `no matching models`) now return HTTP 500 with the message instead of 404/400; the dashboard overlay only checks `res.ok`, so its behavior is unchanged.

- [ ] **Step 6: Run tests + build to verify pass**

Run: `cargo test -p nvr detect::control`
Expected: PASS (4 tests).
Run: `cargo build -p nvr`
Expected: builds clean (fix any unused-import warning-as-error in `api.rs` by deleting the dead `use`).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add nvr/src/detect/control.rs nvr/src/detect/control_test.rs nvr/src/detect/mod.rs nvr/src/detect/api.rs
git commit -m "feat(detect): shared start_tap + reconcile_detection control module"
```

---

### Task 4: Auto-start wiring into the device lifecycle

**Files:**
- Modify: `nvr/src/init/device.rs`
- Modify: `nvr/src/handler/device.rs`

**Interfaces:**
- Consumes: `crate::detect::control::{reconcile_detection, stop_detection}` from Task 3.

This task is integration wiring; correctness of the decision is already covered by `should_auto_start` tests (Task 3). Verified here by build + the reconcile being invoked at every pipe-creating path.

- [ ] **Step 1: Reconcile after the direct-pipe path**

In `nvr/src/init/device.rs`, at the end of `ensure_device_pipe`, the direct-input path currently ends:

```rust
    let config = PipeConfig { input, outputs };
    manager::update_pipe(&device.id, config).await
}
```

Change it to reconcile after the pipe is updated:

```rust
    let config = PipeConfig { input, outputs };
    manager::update_pipe(&device.id, config).await?;
    crate::detect::control::reconcile_detection(device).await;
    Ok(())
}
```

- [ ] **Step 2: Reconcile after the onvif path**

In the same file, the onvif branch currently ends:

```rust
        return manager::upsert_onvif(&device.id, media, cfg, device.include_audio, true).await;
```

Change to:

```rust
        manager::upsert_onvif(&device.id, media, cfg, device.include_audio, true).await?;
        crate::detect::control::reconcile_detection(device).await;
        return Ok(());
```

- [ ] **Step 3: Reconcile after the stream path**

The `stream` branch currently ends:

```rust
        return manager::upsert_stream(
            &device.id,
            media,
            device.input_value.clone(),
            device.include_audio,
            true,
        )
        .await;
```

Change to:

```rust
        manager::upsert_stream(
            &device.id,
            media,
            device.input_value.clone(),
            device.include_audio,
            true,
        )
        .await?;
        crate::detect::control::reconcile_detection(device).await;
        return Ok(());
```

Leave the `xiaomi` and `gb28181` branches unchanged (no subscribable pipe / excluded).

- [ ] **Step 4: Stop detection when a device is removed**

In `nvr/src/handler/device.rs`, in `remove_device`, after `manager::remove_pipe(&id).await?;` add:

```rust
    crate::detect::control::stop_detection(&id);
```

- [ ] **Step 5: Build to verify**

Run: `cargo build -p nvr`
Expected: builds clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add nvr/src/init/device.rs nvr/src/handler/device.rs
git commit -m "feat(detect): auto-start/stop detection from device config on pipe lifecycle"
```

---

### Task 5: Frontend device-config API types

**Files:**
- Modify: `nvr-dashboard/app/src/api/device.ts`

**Interfaces:**
- Produces: `DetectConfig` + `DeviceConfig` TS interfaces; `config?: DeviceConfig` on `DeviceItem` and `DevicePayload`.

- [ ] **Step 1: Add the types and fields**

In `nvr-dashboard/app/src/api/device.ts`, add above `DeviceItem`:

```ts
export interface DetectConfig {
  enabled: boolean
  models: string[]
  sample_every_ms: number
  min_confidence: number
}

export interface DeviceConfig {
  detect?: DetectConfig
}
```

Add `config?: DeviceConfig` to `DeviceItem` (after `record`) and to `DevicePayload` (after `record`).

- [ ] **Step 2: Verify type-check**

Run: `cd nvr-dashboard/app && npm run type-check`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add nvr-dashboard/app/src/api/device.ts
git commit -m "feat(dashboard): DeviceConfig/DetectConfig API types"
```

---

### Task 6: 检测 tab in the device dialog

**Files:**
- Modify: `nvr-dashboard/app/src/views/DeviceListView.vue`

**Interfaces:**
- Consumes: `listDetectModels()` from `src/api/detect.ts`; `DeviceConfig` from `src/api/device.ts` (Task 5).

- [ ] **Step 1: Add imports**

In the `<script setup lang="ts">` of `DeviceListView.vue`, add the PrimeVue tab + control imports. **Note:** `InputNumber` (line 9) and `ToggleSwitch` (line 17) are already imported — do NOT re-import them (duplicate import is an error). Add only:

```ts
import Tabs from "primevue/tabs";
import TabList from "primevue/tablist";
import Tab from "primevue/tab";
import TabPanels from "primevue/tabpanels";
import TabPanel from "primevue/tabpanel";
import MultiSelect from "primevue/multiselect";
import Slider from "primevue/slider";
```

Add a detect-API import `import { listDetectModels } from "../api/detect";` (there is no existing `../api/detect` import in this file), and add `DeviceConfig` to the existing `import type { ... } from "../api/device";`.

- [ ] **Step 2: Add detect state + loader + reset/hydrate**

Add near the other standalone refs (e.g. after the gb picker refs):

```ts
// Detection config uses standalone refs (like the gb/onvif pickers) because it
// is cross-cutting, not a @primevue/forms field. Assembled into config on submit.
const detectEnabled = ref(false);
const detectModels = ref<string[]>([]);
const detectSampleMs = ref(1000);
const detectMinConf = ref(0);
const detectModelOptions = ref<string[]>([]);

function resetDetectFields() {
  detectEnabled.value = false;
  detectModels.value = [];
  detectSampleMs.value = 1000;
  detectMinConf.value = 0;
}

function hydrateDetectFields(device: DeviceItem) {
  const d = device.config?.detect;
  if (!d) {
    resetDetectFields();
    return;
  }
  detectEnabled.value = d.enabled;
  detectModels.value = [...d.models];
  detectSampleMs.value = d.sample_every_ms || 1000;
  detectMinConf.value = d.min_confidence ?? 0;
}
```

In `onMounted`, load the model options once (append inside the existing `onMounted` callback):

```ts
  listDetectModels()
    .then((names) => {
      detectModelOptions.value = names;
    })
    .catch(() => {
      detectModelOptions.value = [];
    });
```

In `openCreateDialog`, add `resetDetectFields();` alongside the existing resets. In `openEditDialog`, add `resetDetectFields();` before hydration and `hydrateDetectFields(device);` after `hydrateGbFields(device);`.

- [ ] **Step 3: Assemble config in `onSubmit`**

In `onSubmit`, after the `const payload: DevicePayload = { ... }` object is built and before `saving.value = true;`, add:

```ts
  if (inputType !== "gb28181") {
    const config: DeviceConfig = {
      detect: {
        enabled: detectEnabled.value,
        models: detectModels.value,
        sample_every_ms: detectSampleMs.value,
        min_confidence: detectMinConf.value,
      },
    };
    payload.config = config;
  }
```

- [ ] **Step 4: Wrap the form body in Tabs and add the 检测 panel**

In the `<template>`, inside `<Form v-slot="$form" ...>`, wrap the existing field content (starting at the first `<div class="field-grid">` that holds `name` + `input_type`, through the last input field — but **not** the submit/cancel action button row) in a `Tabs`. The existing markup goes verbatim inside the `基本` panel; add the new `检测` panel after it. Structure:

```html
      <Form
        v-slot="$form"
        :key="editingDevice?.id ?? 'new'"
        :resolver="resolver"
        :initial-values="formInitialValues"
        class="device-form"
        @submit="onSubmit"
      >
        <Tabs value="basic">
          <TabList>
            <Tab value="basic">基本</Tab>
            <Tab v-if="$form.input_type?.value !== 'gb28181'" value="detect">检测</Tab>
          </TabList>
          <TabPanels>
            <TabPanel value="basic">
              <!-- MOVE the existing form fields here unchanged:
                   the name/input_type field-grid and every
                   `<template v-if $form.input_type ...>` block. -->
            </TabPanel>
            <TabPanel value="detect">
              <div class="field">
                <label for="detect_enabled">启用检测</label>
                <ToggleSwitch v-model="detectEnabled" input-id="detect_enabled" />
              </div>
              <div class="field">
                <label for="detect_models">检测模型</label>
                <MultiSelect
                  v-model="detectModels"
                  input-id="detect_models"
                  class="field-input"
                  :options="detectModelOptions"
                  :disabled="detectModelOptions.length === 0"
                  display="chip"
                  placeholder="全部模型"
                />
                <span v-if="detectModelOptions.length === 0" class="field-hint">
                  未配置检测模型（缺少 models.json）。
                </span>
                <span v-else class="field-hint">留空表示运行全部已配置模型。</span>
              </div>
              <div class="field-grid">
                <div class="field">
                  <label for="detect_sample">抽帧间隔 (ms)</label>
                  <InputNumber
                    v-model="detectSampleMs"
                    input-id="detect_sample"
                    class="field-input"
                    :min="100"
                    :step="100"
                    show-buttons
                  />
                </div>
                <div class="field">
                  <label for="detect_conf">置信度下限：{{ detectMinConf.toFixed(2) }}</label>
                  <Slider
                    v-model="detectMinConf"
                    input-id="detect_conf"
                    class="field-input"
                    :min="0"
                    :max="1"
                    :step="0.05"
                  />
                </div>
              </div>
            </TabPanel>
          </TabPanels>
        </Tabs>

        <!-- KEEP the existing submit/cancel action button row here,
             unchanged, still inside </Form>. -->
      </Form>
```

Leave the submit/cancel button row exactly where it is (after `</Tabs>`, still inside `</Form>`) so `@submit` keeps working.

- [ ] **Step 5: Verify lint + type-check**

Run: `cd nvr-dashboard/app && npm run type-check`
Expected: no errors.
Run: `cd nvr-dashboard/app && npm run lint`
Expected: no ESLint/Stylelint errors (run `npm run lint:style:fix` first if only mechanical style issues remain).

- [ ] **Step 6: Commit**

```bash
git add nvr-dashboard/app/src/views/DeviceListView.vue
git commit -m "feat(dashboard): 检测 tab for per-device detection config"
```

---

## Manual verification (after all tasks)

1. `make run` (or `cargo run -p nvr`) with a `models.json` present in `DETECT_MODELS_DIR`.
2. Add/edit an RTSP device → **检测** tab → enable, pick a model, set sample interval + confidence → save.
3. Confirm the log shows the tap starting for that device id; open the device preview and confirm boxes appear via the existing overlay.
4. Edit the device → disable detection → save → confirm the tap stops.
5. Remove the device → confirm the tap stops.
6. Confirm the 检测 tab is absent for a gb28181 device.

## Self-Review

- **Spec coverage:** Data model (Task 1), auto-start wiring (Tasks 3–4), tap knobs — model subset (Task 3 via `detectors_named`), sample interval (Task 3), min_confidence post-filter (Task 2); frontend tab + controls (Tasks 5–6); reconciliation with overlay (Task 3, API unchanged behavior); error handling — unknown models (`detectors_named` ignores), hub-not-initialized (reconcile warn + empty tab hint), gb28181 exclusion (`should_auto_start`); tests (Tasks 1–3). All spec sections map to a task.
- **Placeholder scan:** none — every code step shows full code; the two "MOVE/KEEP existing markup" notes in Task 6 Step 4 reference verbatim-preserved existing template, not missing content.
- **Type consistency:** `DetectConfig` fields (`enabled`/`models`/`sample_every_ms`/`min_confidence`) match across nvr-db (Task 1), control (Task 3), and TS (Task 5); `start_tap` signature used identically in api.rs (Task 3 Step 5) and reconcile (Task 3 Step 3); `apply_min_confidence` name matches test (Task 2) and tap call.
