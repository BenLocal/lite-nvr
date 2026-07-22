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
  // user may have toggled off during the startDetect round-trip
  if (!active.value) return
  await poll()
  // ...or during the first poll
  if (!active.value) return
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
    if (m.error || !(visible.value[m.name] ?? false)) continue
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
