# Detection overlay on the dashboard preview

Date: 2026-07-22
Status: approved (dashboard draws the detection JSON as colored boxes over the
live preview; all configured models run, per-model color + checkbox toggles
client-side visibility; poll-based, no server-side overlay)

## Problem

The real-time detection feature exposes `GET /api/detect/{pipe}/latest` (a
per-frame, multi-model `FrameResult`) but nothing consumes it yet. We want the
`nvr-dashboard` device preview to draw those detections as boxes over the live
video, so an operator can watch detection happen and visually compare models on
the same frames. This is the frontend half deliberately deferred by the
detection design (`docs/superpowers/specs/2026-07-21-nvr-detect-design.md`).

## Scope

In scope:
- A "检测" toggle in the device preview dialog that starts/stops detection for
  that pipe and shows/hides an overlay.
- An overlay canvas that draws each configured model's boxes in its own color,
  with a legend whose per-model checkboxes toggle client-side visibility.
- A typed API client for the detection endpoints.

Out of scope (unchanged from the detection design):
- Server-side box burn-in / re-encode (its own future spec).
- Push transport (SSE/WebSocket) — this is poll-only.
- Recording annotated video, object tracking/counting, mobile-specific layout.
- A frontend unit-test runner (the dashboard has none; see Testing).

## What the backend already provides

- `POST /api/detect/{pipe}/start` — body `{ "models"?: string[] }` (omitted =
  all configured). Returns plain text (`"started"` / `"already running"` / an
  error string) — NOT the `{code,message,data}` envelope.
- `POST /api/detect/{pipe}/stop` — plain text (`"stopped"` / `"not running"`).
- `GET /api/detect/{pipe}/latest` — the stored `FrameResult` as JSON, or HTTP
  404 when none yet.
- `GET /api/detect/models` — JSON `string[]` of configured model names.
- `FrameResult` shape:
  `{ ts:number, frame_w:number, frame_h:number, models: [ { name:string,
  infer_ms:number, error:string|null, detections: [ { class_id:number,
  label:string, bbox:{x1,y1,x2,y2}, confidence:number } ] } ] }`.
  Bbox coordinates are original-frame pixels in `frame_w × frame_h` space.
- `/api` is session-auth guarded; the client attaches `Authorization: Bearer
  <token>` (same as `src/api/asr.ts`).

## Architecture

Three new units + two edits, mirroring how ASR live subtitles were added
(`src/api/asr.ts` + `TranscriptPanel.vue`, wired into `DeviceListView.vue`).

### New: `src/api/detect.ts`

A raw-`fetch` client (the detect endpoints don't use the shared envelope, so
`request()` doesn't fit — same reasoning as `asr.ts`). Exports:

- Types (mirroring the backend JSON): `BBox { x1,y1,x2,y2:number }`,
  `Detection { class_id:number, label:string, bbox:BBox, confidence:number }`,
  `ModelResult { name:string, infer_ms:number, error:string|null,
  detections:Detection[] }`, `FrameResult { ts:number, frame_w:number,
  frame_h:number, models:ModelResult[] }`.
- `startDetect(pipe: string): Promise<string>` — POST `/detect/{pipe}/start`
  with no body (start all configured models).
- `stopDetect(pipe: string): Promise<string>` — POST `/detect/{pipe}/stop`.
- `getDetectLatest(pipe: string): Promise<FrameResult | null>` — GET
  `/detect/{pipe}/latest`; returns `null` on 404 (no result yet), throws on
  other non-2xx.
- `listDetectModels(): Promise<string[]>` — GET `/detect/models`.

All attach the Bearer token via a local `authHeaders()` helper (copied from
`asr.ts`).

### New: `src/components/DetectionOverlay.vue`

The overlay. Props: `videoEl: HTMLVideoElement | null`, `deviceId: string`,
`active: boolean`. It owns the full detection lifecycle and rendering:

- On `active` → `true`: call `startDetect(deviceId)`, then start an interval
  timer (default **1000 ms**) that calls `getDetectLatest(deviceId)` and stores
  the result. On `active` → `false` or unmount: call `stopDetect(deviceId)`,
  clear the timer, clear the canvas.
- State: `latest: FrameResult | null`; `visible: Record<string, boolean>`
  (per-model, all `true` initially, driven by the legend checkboxes);
  a stable model→color map from a fixed palette
  (`['#22d3ee','#f472b6','#a3e635','#fbbf24','#c084fc','#fb7185']`) keyed by each
  model's position in `listDetectModels()` — so a model keeps its color even on
  a frame where it errored and produced no entry.
- A `<canvas>` absolutely positioned to fill the video's stage
  (`.preview-stage`), redrawn whenever `latest` or `visible` changes and on
  container resize (`ResizeObserver`). The canvas backing store is sized to
  `clientWidth*dpr × clientHeight*dpr` for crisp lines.
- Drawing: for each model with `visible[name]` true and no `error`, draw each
  detection — a 2px stroke rect in the model's color plus a label
  `"{label} {confidence·100|0}%"` in a small filled caption. Coordinates map
  from frame space to the video's on-screen content box via the **object-fit:
  contain** transform (the preview `<video>` is `object-fit: contain`,
  confirmed at `FlvPreviewPlayer.vue`).
- A legend, top-right of the stage: one row per model — a color swatch, a
  checkbox bound to `visible[name]`, the model name, and its live box count.
- Errors: `getDetectLatest` 404 → keep showing nothing (not an error);
  `startDetect` returning an error string (e.g. "no video", "no matching
  models", "no models configured") → surface it as a small dismissible message
  in the corner and leave the overlay empty; transient poll fetch errors →
  swallow and retry on the next tick.

Pure, reviewable core: `frameToScreen(box, frameW, frameH, viewW, viewH)`
returning `{x,y,w,h}` in canvas pixels — the object-fit-contain letterbox math,
kept as a standalone exported function.

### Edit: `src/components/FlvPreviewPlayer.vue`

Add two optional props: `detectDeviceId?: string` and `detectActive?: boolean`.
When `detectDeviceId` is set, render `<DetectionOverlay :video-el="videoRef"
:device-id="detectDeviceId" :active="!!detectActive" />` inside the
`.preview-stage` container (which is `position: relative` and holds the video),
so the overlay shares the video's box. No other player behavior changes; when
the props are absent the player is unchanged.

### Edit: `src/views/DeviceListView.vue`

In the preview dialog, add a "检测" toggle button next to the existing ASR
subtitle toggle, bound to a `detectActive` ref. Pass
`:detect-device-id="previewDevice.id"` and `:detect-active="detectActive"` to
`FlvPreviewPlayer`. Reset `detectActive = false` when the preview dialog closes
or the previewed device changes (so stop fires and the next open starts clean).

## Data flow

```
[检测] toggle on
  → startDetect(deviceId)                      (all configured models run)
  → every ~1s: getDetectLatest(deviceId)
       → FrameResult { frame_w,h, models[ {name, detections[ {label,bbox,conf} ] } ] }
       → assign per-model color; for each visible model:
            frameToScreen(bbox, frame_w,h, stageW,stageH)  (contain letterbox)
            → canvas: stroke rect + label
  legend checkboxes → toggle visible[name] → redraw (no API call)
[检测] toggle off  → stopDetect(deviceId) → clear canvas
```

## Error handling

- No models configured server-side → `startDetect` returns an error string →
  overlay shows it; toggle stays visually on but nothing draws (operator sees
  the reason).
- Pipe not streaming / no video track → `startDetect` "no video" surfaced the
  same way.
- 404 from `/latest` before the first result → draw nothing, keep polling.
- Component unmount / dialog close mid-flight → the timer is cleared and
  `stopDetect` is best-effort (ignore its result/errors).

## Testing

The dashboard has no unit-test runner (only `type-check` + `lint`), so:
- `npm run type-check` and `npm run lint` must pass (the new client + component
  are fully typed).
- `frameToScreen` is a pure exported function; its correctness is verified by
  reasoning + manual check (a box at frame (0,0)-(frame_w,frame_h) must map to
  the full letterboxed content box; a centered box stays centered).
- Manual E2E: run `scripts/detect_e2e.sh` (or start the stack manually) to get a
  live detected stream, open the device preview in the dashboard, toggle 检测,
  and confirm colored boxes track the bus/persons, the legend counts match, and
  unchecking a model hides its color. Toggling off clears the overlay and stops
  detection server-side.

## Conventions

TypeScript, Vue 3 Composition API, PrimeVue components, PascalCase component
filenames — consistent with the rest of `nvr-dashboard/app/src`. The detection
API client mirrors `src/api/asr.ts`; the overlay wiring mirrors the ASR
`TranscriptPanel` integration in `DeviceListView.vue`.
