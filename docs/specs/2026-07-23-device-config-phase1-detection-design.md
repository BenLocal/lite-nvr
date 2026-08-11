# Device-centric configuration — Phase 1: per-device Detection

Date: 2026-07-23
Status: Design approved (pending spec review)

## Context & motivation

Several backend subsystems in lite-nvr are fully built but not exposed to the
user as configuration. Rather than add a separate top-level page per subsystem,
we make the **device add/edit dialog** the single place to configure per-device
behaviour. Precedent already exists: GB28181 and ONVIF are managed there today
(input types with channel pickers + PTZ).

This is the umbrella of **device-centric configuration**, delivered in phases.
Each phase adds one tab to the device dialog plus its runtime wiring, and each
phase gets its own spec → plan → build cycle.

### Phased roadmap (order = value × low-backend-risk)

1. **Detection** — *this spec*. Highest visible value, lowest backend risk
   (hub/tap/config already exist). Also builds the shared **tabbed-form shell**
   and the **per-device config storage** pattern that phases 2–4 reuse.
2. **ONVIF/GB stream params** — profile/substream selection. ONVIF
   `profile_token` is already plumbed into `stream_uri()`.
3. **Transport routing** — per-device target selection; requires a
   targets-CRUD surface (Settings section) + a per-device→target mapping +
   worker filter (worker is currently fully global).
4. **Recording** — per-device retention override (medium) and/or a record
   schedule (no backend today; largest new work). Last.

Phases 2–4 are **out of scope** for this spec and will be brainstormed
separately, reusing the shell built here.

## Phase 1 scope

**In scope:** persist a per-device detection config; auto-start/stop the
detection tap from that config when a device's pipe is (re)built; a **检测**
tab in the device dialog to edit it; the reusable tabbed-form shell and the
`DeviceConfig` JSON storage pattern.

**Out of scope / explicit boundaries:**

- Detection applies only to manager `Entry::Pipe` inputs:
  `net`/`rtsp`/`rtmp`/`file`/`v4l2`/`x11grab`/`lavfi`. Native supervisors and
  workers (`gb28181`, `onvif`, `stream`, `xiaomi`) do not expose a subscribable
  pipe. The backend publishes this list from `GET /api/detect/capabilities`;
  the dashboard uses that response instead of maintaining a second whitelist.
- No detection **persistence of results** and no **alarm/event** system — those
  are separate (category-B) work, not part of this phase.

## 1. Data model

Devices are stored in the KV table (`kvs`, `module = "device"`) as JSON —
there is no device table — so extending the model needs **no migration**.

Extend `DeviceInfo` (`nvr-db/src/device.rs`) with one backward-compatible
field:

```rust
#[serde(default)]
pub config: DeviceConfig,
```

```rust
#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct DeviceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect: Option<DetectConfig>,
    // phases 2–4 add: stream, transport, recording
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DetectConfig {
    pub enabled: bool,
    /// Subset of configured model names to run. Empty = all configured models.
    #[serde(default)]
    pub models: Vec<String>,
    /// Sampling interval in ms. Falls back to the hub default when 0/absent.
    #[serde(default)]
    pub sample_every_ms: u64,
    /// Post-inference confidence floor. 0.0 = keep each model's built-in `conf`.
    #[serde(default)]
    pub min_confidence: f32,
}
```

Existing device rows lack `config`; `#[serde(default)]` deserializes them with
`config: DeviceConfig::default()` (`detect: None` → detection off). Verified
back-compat path: `nvr_db::device::{list,get}` call `serde_json::from_str`.

## 2. Backend design

### Persist

`DevicePayload` (`nvr/src/handler/device.rs`) gains an optional `config`
(defaulted). `add_device`/`update_device` carry it into `DeviceInfo` before
`upsert`. No other handler changes.

### Auto-start / stop

`ensure_device_pipe(device)` (`nvr/src/init/device.rs`) is the single choke
point — called at startup (over every device) and on add/update. After the pipe
is ensured, reconcile detection:

- If `device.config.detect` is `Some(cfg)` with `cfg.enabled` **and** the
  device is pipe-backed: start the tap for `device.id` using the device's model
  subset, `sample_every_ms`, and `min_confidence`.
- Otherwise: stop the tap for `device.id` (idempotent no-op if not running).

`DetectHub` is initialized synchronously before the ZLM/device/API tasks are
spawned and is passed explicitly to each consumer. This guarantees startup
reconciliation always sees the hub. A manual stop cancels both the running tap
and any pending auto-start generation, so a delayed retry cannot restart it.

### Tap knobs

- **Sample interval:** `tap::run` already accepts `sample_interval_ms`; pass the
  device value, falling back to `hub.sample_interval_ms()` when 0.
- **Model subset:** filter `DetectorConfig`s by `cfg.models` (mirrors the
  existing `StartBody.models` semantics). Unknown names are logged and ignored;
  a mixed list runs its valid subset, while an unknown-only/empty resolved set
  falls back to all configured models for historical-config compatibility.
  New API/persisted input is trimmed and deduplicated, with at most 32 names
  and 128 characters per name. These limits are returned by the capabilities
  endpoint together with the configured model list.
- **Sample interval validation:** `0` means the hub default; explicit values
  are bounded to `1..=3_600_000` ms (one hour). The same maximum is returned by
  the capabilities endpoint and enforced by the form resolver.
- **min_confidence:** applied as a **post-inference filter** — drop detections
  below the floor before `hub.store`. Chosen over rebuilding models per device
  (which would re-instantiate the ONNX session per device); the filter is O(n)
  over detections and needs no model rebuild. *(Decision (a), approved.)*

### Relationship to the manual `/detect/{pipe}/start` API

Device config is the source of truth for **auto-start**; the overlay remains a
**live visualization + manual toggle**. Both drive the same idempotent hub, so
they coexist: a device with persistent detection is simply already running when
its preview opens. The overlay owns a tap only when its start request returns
`started` together with the opaque `x-detection-tap-lease` response header. It
includes that lease when stopping; the hub stops the tap only if the lease still
matches the current epoch. Therefore an old preview cannot stop a replacement
tap. `already running` and persisted detection are observation-only, so closing
the preview never stops a tap owned by device configuration. An unleased manual
stop remains authoritative: it cancels pending auto-start and the current tap.
*(Decision (b), approved — manual toggle retained.)*

## 3. Frontend design

### Tabbed-form shell (reusable)

Convert the device dialog's `@primevue/forms` `Form` body into a PrimeVue
`Tabs`:

- **基本** — all current fields (name, type, value, description, audio, record,
  plus the existing GB/ONVIF/Xiaomi structured sub-forms).
- **检测** — new (this phase). Phases 2–4 add their own tabs beside it.

The 检测 tab is shown only for input types returned by the backend capabilities
endpoint.

### 检测 tab controls

Bound to a `config.detect` structure, following the existing "structured
sub-fields serialized on submit" precedent (as Xiaomi/ONVIF fields already do):

- `enabled` — `@primevue/forms` ToggleSwitch field.
- `models` — `@primevue/forms` MultiSelect populated from the capabilities
  response. A successful empty list shows "未配置检测模型"; request failure has
  its own error and retry state.
- `sample_every_ms` — `@primevue/forms` InputNumber (default 1000, `0` means
  server default, maximum 3,600,000).
- `min_confidence` — Slider 0.0–1.0 (default 0.0 = model default).

All four controls participate in the shared form resolver and render field
errors. On submit, assemble a new payload/config/model array without mutating
the form or prior payload.

## 4. Error handling

- Unknown model names in `config.detect.models` → ignored server-side with a
  log; the valid subset runs. Empty resolved set → treat as "all".
- Missing `models.json` is represented by a successful capabilities response
  with an empty model list; transport/server failures are shown separately and
  can be retried.
- Detection errors never break the pipe — the tap already isolates model
  failures per frame.
- Non-pipe-backed device (gb28181) with a stale `detect.enabled` → auto-start
  skipped; the tab is hidden so this shouldn't be reachable via the UI.

## 5. Testing

- **nvr-db** (`device_test.rs`): `DeviceInfo` round-trips both with and without
  `config` (back-compat); a row missing `config` deserializes to
  `detect: None`.
- **handler/device** (`device_test.rs`): add/update persists `config.detect`.
- **init/device**: unit-test the auto-start decision — `enabled` +
  pipe-backed → start; `enabled=false` or gb28181 → stop/skip.
- **detect/tap/hub/api**: `min_confidence` post-filter drops
  detections below the floor; model-subset filtering selects the right
  detectors; stale tap/overlay leases cannot stop a replacement generation.
- **Frontend**: Vitest covers form validation, immutable payload construction,
  capability error state, and overlay ownership; Playwright covers the
  capability failure/retry flow; type-check and lint remain required.

## 6. Approved decisions

- **(a)** `min_confidence` is a post-inference filter, not a per-device model
  rebuild.
- **(b)** The manual overlay start/stop toggle is retained alongside persisted
  auto-start.

## 7. Deliverables

- `nvr-db`: `DeviceConfig` / `DetectConfig` types; `DeviceInfo.config` field.
- `nvr`: payload plumbing in `handler/device.rs`; auto-start reconciliation in
  `init/device.rs`; tap model-subset + `min_confidence` post-filter + per-tap
  sample interval.
- `nvr-dashboard`: tabbed device dialog; 检测 tab; `device.ts` payload carries
  `config`.
- Tests as in §5.
