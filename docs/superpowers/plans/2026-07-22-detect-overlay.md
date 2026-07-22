# Detection Overlay on Dashboard Preview — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Draw the detection JSON (`GET /api/detect/{pipe}/latest`) as colored, per-model boxes over the device live preview in `nvr-dashboard`, with a "检测" toggle and per-model visibility checkboxes.

**Architecture:** A typed raw-fetch API client (`src/api/detect.ts`, mirroring `src/api/asr.ts`), a pure coordinate/palette util (`src/utils/detectOverlay.ts`), and a self-contained `DetectionOverlay.vue` component (its own toggle button, start/stop, ~1s polling, a `<canvas>` overlay, and a legend). The overlay is rendered inside `FlvPreviewPlayer.vue`'s `.preview-media` stage (gated by a new optional `detectDeviceId` prop) and wired in from `DeviceListView.vue`'s preview dialog.

**Tech Stack:** Vue 3 Composition API (`<script setup lang="ts">`), TypeScript, PrimeVue (Button, Checkbox), HTML Canvas 2D.

## Global Constraints

- TypeScript + Vue 3 Composition API; PrimeVue components; PascalCase component filenames — consistent with `nvr-dashboard/app/src`.
- The detection API client mirrors `src/api/asr.ts`: raw `fetch` (these endpoints return plain text / bare JSON, NOT the `{code,message,data}` envelope), Bearer token from `getAuthToken()` (`src/auth/token.ts`).
- `FrameResult` JSON shape (from the backend): `{ ts:number, frame_w:number, frame_h:number, models: [ { name:string, infer_ms:number, error:string|null, detections: [ { class_id:number, label:string, bbox:{x1,y1,x2,y2}, confidence:number } ] } ] }`. Bbox coords are original-frame pixels in `frame_w × frame_h` space.
- `GET /api/detect/{pipe}/latest` returns HTTP 404 when there is no result yet → client returns `null`.
- All configured models run when detection starts (`startDetect` sends no model subset). Per-model color is assigned by the model's index in `listDetectModels()` order (stable). The legend checkboxes toggle **client-side visibility only** (no API call).
- Coordinate mapping uses the frame's own `frame_w/frame_h` as the source space and the `.preview-media` stage's `clientWidth/clientHeight` as the destination, with an **object-fit: contain** letterbox transform (the preview video is `object-fit: contain`). This is backend-agnostic (works whether the active player is mpegts `<video>` or jessibuca canvas).
- Poll interval: **1000 ms**. The overlay `<canvas>` is `pointer-events: none` (so the video's native controls stay clickable); the toggle button, legend, and error message are `pointer-events: auto`.
- No frontend unit-test runner exists (only `type-check` + `lint`). Verification per task = `npm run type-check` + `npm run lint` (+ `npm run build` at the end) + manual E2E. Run all npm commands from `nvr-dashboard/app`.

**Refinements vs. the spec** (cleaner realizations of the same approved behavior, noted for transparency): the "检测" toggle lives *inside* the self-contained `DetectionOverlay` (mirroring how `TranscriptPanel` self-contains its controls) rather than a `DeviceListView`-owned `detectActive` prop; and the overlay measures the `.preview-media` stage and maps from `FrameResult.frame_w/frame_h`, so it does not depend on the `<video>` element (the spec's `videoEl` prop is dropped).

---

### Task 1: API client + pure overlay util

**Files:**
- Create: `nvr-dashboard/app/src/api/detect.ts`
- Create: `nvr-dashboard/app/src/utils/detectOverlay.ts`

**Interfaces:**
- Produces (consumed by Task 2):
  - `detect.ts`: types `BBox`, `Detection`, `ModelResult`, `FrameResult`; functions `startDetect(pipe:string):Promise<string>`, `stopDetect(pipe:string):Promise<string>`, `getDetectLatest(pipe:string):Promise<FrameResult|null>`, `listDetectModels():Promise<string[]>`.
  - `detectOverlay.ts`: `DETECT_PALETTE:string[]`, `colorForIndex(i:number):string`, `ScreenRect`, `frameToScreen(box, frameW, frameH, viewW, viewH):ScreenRect`.

- [ ] **Step 1: Write `src/api/detect.ts`**

```ts
import { getAuthToken } from '../auth/token'

// Detection control + read endpoints return plain text (start/stop) or bare
// JSON (latest/models), not the shared `{ code, message, data }` envelope, so
// they use raw fetch — same pattern as `src/api/asr.ts`.

export interface BBox {
  x1: number
  y1: number
  x2: number
  y2: number
}

export interface Detection {
  class_id: number
  label: string
  bbox: BBox
  confidence: number
}

export interface ModelResult {
  name: string
  infer_ms: number
  error: string | null
  detections: Detection[]
}

export interface FrameResult {
  ts: number
  frame_w: number
  frame_h: number
  models: ModelResult[]
}

function authHeaders(): Headers {
  const token = getAuthToken()
  const headers = new Headers()
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  return headers
}

/** Start detection for a pipe (all configured models run). Status string. */
export async function startDetect(pipe: string): Promise<string> {
  const res = await fetch(`/api/detect/${encodeURIComponent(pipe)}/start`, {
    method: 'POST',
    headers: authHeaders(),
  })
  const text = await res.text().catch(() => '')
  if (!res.ok) {
    throw new Error(text || `请求失败 (${res.status})`)
  }
  return text
}

/** Stop detection for a pipe. Status string. */
export async function stopDetect(pipe: string): Promise<string> {
  const res = await fetch(`/api/detect/${encodeURIComponent(pipe)}/stop`, {
    method: 'POST',
    headers: authHeaders(),
  })
  const text = await res.text().catch(() => '')
  if (!res.ok) {
    throw new Error(text || `请求失败 (${res.status})`)
  }
  return text
}

/** Latest per-frame, multi-model result — or `null` if none yet (HTTP 404). */
export async function getDetectLatest(pipe: string): Promise<FrameResult | null> {
  const res = await fetch(`/api/detect/${encodeURIComponent(pipe)}/latest`, {
    headers: authHeaders(),
  })
  if (res.status === 404) {
    return null
  }
  if (!res.ok) {
    throw new Error(`请求失败 (${res.status})`)
  }
  return (await res.json()) as FrameResult
}

/** Names of the configured detection models. */
export async function listDetectModels(): Promise<string[]> {
  const res = await fetch('/api/detect/models', { headers: authHeaders() })
  if (!res.ok) {
    throw new Error(`请求失败 (${res.status})`)
  }
  return (await res.json()) as string[]
}
```

- [ ] **Step 2: Write `src/utils/detectOverlay.ts`**

```ts
// Pure helpers for the detection overlay: a stable color palette and the
// frame-space -> screen-space mapping under `object-fit: contain`.

export const DETECT_PALETTE = [
  '#22d3ee', // cyan
  '#f472b6', // pink
  '#a3e635', // lime
  '#fbbf24', // amber
  '#c084fc', // violet
  '#fb7185', // rose
]

/** Stable color for a model by its index in the configured-model list. */
export function colorForIndex(i: number): string {
  const n = DETECT_PALETTE.length
  return DETECT_PALETTE[((i % n) + n) % n]
}

export interface ScreenRect {
  x: number
  y: number
  w: number
  h: number
}

/**
 * Map a box in frame-pixel space (`frameW × frameH`) to on-screen CSS pixels
 * within a `viewW × viewH` element whose content is displayed with
 * `object-fit: contain` (aspect preserved, letterboxed, centered).
 *
 * Example: box (100,100)-(200,200) in a 800×600 frame shown in a 400×400 view
 * → scale = min(400/800, 400/600) = 0.5; offsetY = (400 - 600*0.5)/2 = 50
 * → { x: 50, y: 100, w: 50, h: 50 }.
 */
export function frameToScreen(
  box: { x1: number; y1: number; x2: number; y2: number },
  frameW: number,
  frameH: number,
  viewW: number,
  viewH: number,
): ScreenRect {
  if (frameW <= 0 || frameH <= 0) {
    return { x: 0, y: 0, w: 0, h: 0 }
  }
  const scale = Math.min(viewW / frameW, viewH / frameH)
  const offsetX = (viewW - frameW * scale) / 2
  const offsetY = (viewH - frameH * scale) / 2
  return {
    x: box.x1 * scale + offsetX,
    y: box.y1 * scale + offsetY,
    w: (box.x2 - box.x1) * scale,
    h: (box.y2 - box.y1) * scale,
  }
}
```

- [ ] **Step 3: Type-check + lint**

Run (from `nvr-dashboard/app`): `npm run type-check && npm run lint`
Expected: PASS (both files fully typed; no lint errors). If `lint` reports pre-existing issues in unrelated files, they are not yours — confirm the two new files are clean.

- [ ] **Step 4: Commit**

```bash
git add nvr-dashboard/app/src/api/detect.ts nvr-dashboard/app/src/utils/detectOverlay.ts
git commit -m "feat(dashboard): detection API client + overlay coordinate util"
```

---

### Task 2: `DetectionOverlay.vue` component

**Files:**
- Create: `nvr-dashboard/app/src/components/DetectionOverlay.vue`

**Interfaces:**
- Consumes: `startDetect`, `stopDetect`, `getDetectLatest`, `listDetectModels`, `FrameResult` from `../api/detect`; `colorForIndex`, `frameToScreen` from `../utils/detectOverlay`.
- Produces (consumed by Task 3): a component with a single required prop `deviceId: string`. It self-contains the toggle, lifecycle, polling, canvas, and legend. It expects to be mounted inside a `position: relative` container it should fill.

- [ ] **Step 1: Write `src/components/DetectionOverlay.vue`**

```vue
<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Button from 'primevue/button'
import Checkbox from 'primevue/checkbox'
import {
  getDetectLatest,
  listDetectModels,
  startDetect,
  stopDetect,
  type FrameResult,
} from '../api/detect'
import { colorForIndex, frameToScreen } from '../utils/detectOverlay'

const props = defineProps<{ deviceId: string }>()

const POLL_MS = 1000

const rootRef = ref<HTMLDivElement | null>(null)
const canvasRef = ref<HTMLCanvasElement | null>(null)

const active = ref(false)
const latest = ref<FrameResult | null>(null)
const models = ref<string[]>([])
const visible = ref<Record<string, boolean>>({})
const errorMsg = ref('')

let timer: ReturnType<typeof setInterval> | undefined
let resizeObs: ResizeObserver | undefined

const colorOf = (name: string) => colorForIndex(models.value.indexOf(name))
const countOf = (name: string) =>
  latest.value?.models.find((m) => m.name === name)?.detections.length ?? 0

const legend = computed(() =>
  models.value.map((name) => ({ name, color: colorOf(name), count: countOf(name) })),
)

async function loadModels() {
  try {
    models.value = await listDetectModels()
    const v: Record<string, boolean> = {}
    for (const m of models.value) v[m] = true
    visible.value = v
  } catch {
    models.value = []
  }
}

function stopPolling() {
  if (timer) {
    clearInterval(timer)
    timer = undefined
  }
}

async function poll() {
  try {
    latest.value = await getDetectLatest(props.deviceId)
  } catch {
    // transient fetch error — keep the last frame, retry next tick
  }
}

async function start() {
  errorMsg.value = ''
  try {
    await startDetect(props.deviceId)
  } catch (e) {
    errorMsg.value = e instanceof Error ? e.message : String(e)
    active.value = false
    return
  }
  await poll()
  stopPolling()
  timer = setInterval(poll, POLL_MS)
}

async function stop() {
  stopPolling()
  latest.value = null
  try {
    await stopDetect(props.deviceId)
  } catch {
    // best effort
  }
  draw()
}

function toggle() {
  active.value = !active.value
}

watch(active, (on) => {
  if (on) start()
  else stop()
})

watch([latest, visible], () => draw(), { deep: true })

function draw() {
  const canvas = canvasRef.value
  const root = rootRef.value
  if (!canvas || !root) return
  const w = root.clientWidth
  const h = root.clientHeight
  const dpr = window.devicePixelRatio || 1
  canvas.width = Math.max(1, Math.round(w * dpr))
  canvas.height = Math.max(1, Math.round(h * dpr))
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, w, h)

  const fr = latest.value
  if (!fr) return
  ctx.lineWidth = 2
  ctx.font = '12px system-ui, sans-serif'
  ctx.textBaseline = 'top'

  for (const m of fr.models) {
    if (m.error || !visible.value[m.name]) continue
    const color = colorOf(m.name)
    ctx.strokeStyle = color
    ctx.fillStyle = color
    for (const d of m.detections) {
      const r = frameToScreen(d.bbox, fr.frame_w, fr.frame_h, w, h)
      ctx.strokeRect(r.x, r.y, r.w, r.h)
      const label = `${d.label} ${Math.round(d.confidence * 100)}%`
      const tw = ctx.measureText(label).width + 6
      const ty = r.y - 15 >= 0 ? r.y - 15 : r.y
      ctx.fillStyle = color
      ctx.fillRect(r.x, ty, tw, 15)
      ctx.fillStyle = '#0b0f1a'
      ctx.fillText(label, r.x + 3, ty + 2)
      ctx.fillStyle = color
    }
  }
}

onMounted(() => {
  loadModels()
  resizeObs = new ResizeObserver(() => draw())
  if (rootRef.value) resizeObs.observe(rootRef.value)
})

onBeforeUnmount(() => {
  stopPolling()
  if (resizeObs) resizeObs.disconnect()
  // stop detection server-side if it was running
  if (active.value) {
    stopDetect(props.deviceId).catch(() => {})
  }
})
</script>

<template>
  <div ref="rootRef" class="detect-overlay">
    <canvas ref="canvasRef" class="detect-canvas" />

    <Button
      class="detect-toggle"
      size="small"
      :severity="active ? 'success' : 'secondary'"
      :icon="active ? 'pi pi-eye' : 'pi pi-eye-slash'"
      label="检测"
      @click="toggle"
    />

    <div v-if="active && legend.length" class="detect-legend">
      <label v-for="item in legend" :key="item.name" class="detect-legend-row">
        <Checkbox v-model="visible[item.name]" :binary="true" />
        <span class="detect-swatch" :style="{ backgroundColor: item.color }" />
        <span class="detect-legend-name">{{ item.name }}</span>
        <span class="detect-legend-count">{{ item.count }}</span>
      </label>
    </div>

    <div v-if="active && errorMsg" class="detect-error">
      {{ errorMsg }}
      <button type="button" class="detect-error-x" @click="errorMsg = ''">×</button>
    </div>
  </div>
</template>

<style scoped>
.detect-overlay {
  position: absolute;
  inset: 0;
  pointer-events: none;
  z-index: 4;
}
.detect-canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}
.detect-toggle {
  position: absolute;
  top: 0.5rem;
  left: 0.5rem;
  pointer-events: auto;
  opacity: 0.9;
}
.detect-legend {
  position: absolute;
  top: 0.5rem;
  right: 0.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding: 0.4rem 0.55rem;
  border-radius: 0.5rem;
  background: rgb(11 15 26 / 65%);
  pointer-events: auto;
}
.detect-legend-row {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  color: white;
  font-size: 0.8rem;
  cursor: pointer;
}
.detect-swatch {
  width: 0.75rem;
  height: 0.75rem;
  border-radius: 0.2rem;
  display: inline-block;
}
.detect-legend-name {
  min-width: 4.5rem;
}
.detect-legend-count {
  opacity: 0.7;
  margin-left: auto;
}
.detect-error {
  position: absolute;
  left: 0.5rem;
  bottom: 0.5rem;
  max-width: calc(100% - 1rem);
  padding: 0.35rem 0.6rem;
  border-radius: 0.5rem;
  background: rgb(190 30 30 / 85%);
  color: white;
  font-size: 0.8rem;
  pointer-events: auto;
}
.detect-error-x {
  margin-left: 0.5rem;
  background: transparent;
  border: none;
  color: white;
  cursor: pointer;
}
</style>
```

- [ ] **Step 2: Type-check + lint**

Run (from `nvr-dashboard/app`): `npm run type-check && npm run lint`
Expected: PASS. Common gotchas to fix if they surface: the `Checkbox v-model="visible[item.name]"` binding must type-check against `Record<string, boolean>` (it does, since `visible[name]` is `boolean`); the PrimeVue `Button`/`Checkbox` import paths are `primevue/button` / `primevue/checkbox` (matching `FlvPreviewPlayer.vue`'s `primevue/button`).

- [ ] **Step 3: Commit**

```bash
git add nvr-dashboard/app/src/components/DetectionOverlay.vue
git commit -m "feat(dashboard): DetectionOverlay — toggle, polling, canvas boxes, legend"
```

---

### Task 3: Wire the overlay into the preview

**Files:**
- Modify: `nvr-dashboard/app/src/components/FlvPreviewPlayer.vue` (add `detectDeviceId` prop; render the overlay in `.preview-media`)
- Modify: `nvr-dashboard/app/src/views/DeviceListView.vue` (pass `:detect-device-id` in the preview dialog)

**Interfaces:**
- Consumes: `DetectionOverlay` (Task 2), which needs a `deviceId: string` prop and a `position: relative` parent (the existing `.preview-media`).

- [ ] **Step 1: Add the prop + import to `FlvPreviewPlayer.vue`**

At the top of `<script setup lang="ts">`, add the import next to the existing imports:

```ts
import DetectionOverlay from './DetectionOverlay.vue'
```

Change the `defineProps` from:

```ts
const props = defineProps<{
  url: string
}>()
```

to:

```ts
const props = defineProps<{
  url: string
  detectDeviceId?: string
}>()
```

- [ ] **Step 2: Render the overlay inside `.preview-media`**

In the template, the `.preview-media` block currently is:

```html
      <div class="preview-media">
        <video
          ref="videoRef"
          class="preview-video"
          controls
          autoplay
          muted
          playsinline
        />
        <div ref="jessibucaRef" class="preview-jessibuca" />
      </div>
```

Add the overlay as the last child of `.preview-media` (it is `position: relative`, so the overlay's `position: absolute; inset: 0` fills it):

```html
      <div class="preview-media">
        <video
          ref="videoRef"
          class="preview-video"
          controls
          autoplay
          muted
          playsinline
        />
        <div ref="jessibucaRef" class="preview-jessibuca" />
        <DetectionOverlay
          v-if="detectDeviceId"
          :key="detectDeviceId"
          :device-id="detectDeviceId"
        />
      </div>
```

(The `:key="detectDeviceId"` remounts the overlay fresh when the previewed device changes, so it re-loads models and resets its toggle.)

- [ ] **Step 3: Pass the device id from `DeviceListView.vue`**

In the preview dialog, the `<FlvPreviewPlayer>` usage is currently:

```html
        <FlvPreviewPlayer
          :url="previewDevice?.flv_url || (previewDevice ? buildFlvUrl(previewDevice.id) : '')"
        />
```

Add the `detect-device-id` binding:

```html
        <FlvPreviewPlayer
          :url="previewDevice?.flv_url || (previewDevice ? buildFlvUrl(previewDevice.id) : '')"
          :detect-device-id="previewDevice?.id"
        />
```

(When `previewDevice` is null the id is `undefined` and the overlay's `v-if` keeps it unmounted. Closing the dialog unmounts `FlvPreviewPlayer` → the overlay's `onBeforeUnmount` stops detection.)

- [ ] **Step 4: Type-check, lint, build**

Run (from `nvr-dashboard/app`): `npm run type-check && npm run lint && npm run build`
Expected: all PASS. `npm run build` produces `dist/` with no errors.

- [ ] **Step 5: Manual E2E (documented; needs a running stack with a model)**

With a real detection stack running (`bash scripts/detect_e2e.sh` gives one, or start the pieces manually with `ORT_DYLIB_PATH` + `DETECT_MODELS_DIR` set and a device streaming), open the dashboard, open the device's 实时预览, and click 检测:
1. Colored boxes appear over the video within a few seconds and track the objects; the legend lists each model with its live count.
2. Unchecking a model in the legend hides its color immediately (no reload); re-checking restores it.
3. Clicking 检测 again clears the overlay; the server-side detection stops (verify via `GET /api/detect/{id}/latest` no longer updating, or the nvr log).
4. Closing the preview dialog stops detection.
5. The video's native controls (play/pause/volume) remain clickable through the overlay.

Record the result in the task report; if no model/`.so` is available in the environment, note that the manual E2E was not run and the build/type-check/lint gates are the automated coverage.

- [ ] **Step 6: Commit**

```bash
git add nvr-dashboard/app/src/components/FlvPreviewPlayer.vue nvr-dashboard/app/src/views/DeviceListView.vue
git commit -m "feat(dashboard): show detection overlay in device preview"
```

---

## Notes for the executor

- **Run npm from `nvr-dashboard/app`.** Scripts: `type-check` = `vue-tsc --build`, `lint` = `run-s lint:eslint lint:style`, `build` = the production build.
- **No unit-test runner.** Do not add vitest/jest — the repo has none and the spec says so. The pure `frameToScreen`/`colorForIndex` are verified by the worked example in Task 1 and the manual E2E; the rest by type-check + lint + build.
- **`.preview-media` is `position: relative`** (confirmed in `FlvPreviewPlayer.vue`), which anchors the overlay. Do not change its positioning.
- **Canvas must stay `pointer-events: none`** so the `<video controls>` underneath stays usable; only the toggle/legend/error are interactive.
- **Backend-agnostic mapping:** coordinates come from `FrameResult.frame_w/frame_h`, not the `<video>` intrinsic size, so the overlay works for both the mpegts `<video>` and jessibuca-canvas player backends without change.
