<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import { listDevices, type DeviceItem } from '../api/device'
import {
  createCompositor,
  listCompositors,
  relayoutCompositor,
  removeCompositor,
  switchRegion,
  type CompositorProgram,
  type CompositorRegion,
} from '../api/compositor'
import { ensureFlvJs, type FlvPlayer } from '../utils/flvjs'
import SwitcherTile from '../components/SwitcherTile.vue'

interface LayoutDef {
  id: string
  label: string
  cells: number
  cls: string
}

interface Rect {
  x: number
  y: number
  w: number
  h: number
}

const SINGLE_LAYOUT: LayoutDef = { id: 'single', label: '单画面', cells: 1, cls: 'single' }

const LAYOUTS: LayoutDef[] = [
  SINGLE_LAYOUT,
  { id: 'side2', label: '双画面', cells: 2, cls: 'side2' },
  { id: 'pip', label: '画中画', cells: 2, cls: 'pip' },
  { id: 'quad', label: '四宫格', cells: 4, cls: 'quad' },
  { id: 'oneplusfive', label: '1 + 5', cells: 6, cls: 'oneplusfive' },
  { id: 'nine', label: '九宫格', cells: 9, cls: 'nine' },
]

// Server compositor canvas. The frontend maps each layout to pixel regions at
// this resolution so the composited program matches the on-screen grid.
const CANVAS_W = 1280
const CANVAS_H = 720
const CANVAS_FPS = 25
// One long-lived program per dashboard. Its regions map 1:1 to the layout cells
// (region index === cell index), so a source change is a live per-region switch
// that never restarts the published stream. Only a layout change re-creates it.
const PROGRAM_ID = 'director'

const sources = ref<DeviceItem[]>([])
const loading = ref(false)
const error = ref('')
const layoutId = ref('quad')
const assignments = ref<(string | null)[]>([])
const activeCell = ref(0)
const beforeEnlarge = ref<string | null>(null)

const program = ref<CompositorProgram | null>(null)
const programBusy = ref(false)
const programError = ref('')
const programVideo = ref<HTMLVideoElement | null>(null)
let programPlayer: FlvPlayer | null = null
const copiedKey = ref('')
let copyTimer: ReturnType<typeof setTimeout> | undefined

const currentLayout = computed(() => LAYOUTS.find((l) => l.id === layoutId.value) ?? SINGLE_LAYOUT)
const isLive = computed(() => !!program.value)

function sourceById(id: string | null): DeviceItem | null {
  if (!id) {
    return null
  }
  return sources.value.find((s) => s.id === id) ?? null
}

// Fill any empty slot with a not-yet-shown source so the wall comes up populated.
function autoFill(slots: (string | null)[]): (string | null)[] {
  const used = new Set(slots.filter((v): v is string => !!v))
  const pool = sources.value.filter((s) => !used.has(s.id)).map((s) => s.id)
  let next = 0
  return slots.map((v) => {
    if (v) {
      return v
    }
    const picked = next < pool.length ? pool[next++] : undefined
    return picked ?? null
  })
}

function blankSlots(count: number): (string | null)[] {
  return Array.from({ length: count }, () => null)
}

async function loadSources() {
  loading.value = true
  error.value = ''
  try {
    const list = await listDevices()
    sources.value = Array.isArray(list) ? list : []
    assignments.value = autoFill(blankSlots(currentLayout.value.cells))
    activeCell.value = 0
    beforeEnlarge.value = null
  } catch (e) {
    error.value = e instanceof Error ? e.message : '加载设备失败'
  } finally {
    loading.value = false
  }
}

async function setLayout(id: string) {
  const next = LAYOUTS.find((l) => l.id === id)
  if (!next) {
    return
  }
  // Preserve the overlapping slots, auto-fill the newly exposed ones.
  const kept = Array.from({ length: next.cells }, (_, i) => assignments.value[i] ?? null)
  assignments.value = autoFill(kept)
  layoutId.value = id
  if (activeCell.value >= next.cells) {
    activeCell.value = 0
  }
  beforeEnlarge.value = null
  // A layout change is applied live: the server rebuilds only its filter graph
  // and keeps the same encoder/muxer, so the published stream never restarts.
  if (program.value) {
    await relayoutProgram()
  }
}

function selectCell(index: number) {
  activeCell.value = index
}

async function assignToActive(source: DeviceItem) {
  const cell = activeCell.value
  const slots = assignments.value.slice()
  slots[cell] = source.id
  assignments.value = slots
  // Jump to the next empty slot so several sources can be placed in a row.
  const nextEmpty = slots.findIndex((v, i) => i > cell && !v)
  if (nextEmpty !== -1) {
    activeCell.value = nextEmpty
  }
  if (program.value) {
    await liveSwitch(cell, source.id)
  }
}

async function clearCell(index: number) {
  const slots = assignments.value.slice()
  slots[index] = null
  assignments.value = slots
  activeCell.value = index
  if (program.value) {
    await liveSwitch(index, '')
  }
}

// Double-click a tile to blow it up to a single view; double-click again (or
// pick another layout) to return to where you were.
async function enlargeCell(index: number) {
  if (layoutId.value === 'single') {
    if (beforeEnlarge.value) {
      const back = beforeEnlarge.value
      await setLayout(back)
    }
    return
  }
  const prev = layoutId.value
  const src = assignments.value[index] ?? null
  layoutId.value = 'single'
  assignments.value = [src]
  activeCell.value = 0
  beforeEnlarge.value = prev
  if (program.value) {
    await relayoutProgram()
  }
}

function isAssigned(id: string): boolean {
  return assignments.value.includes(id)
}

// ---- Server compositor (PROGRAM output) ---------------------------------

// Per-layout pixel regions on the CANVAS_W×CANVAS_H canvas, in cell order.
function cellRects(layout: LayoutDef): Rect[] {
  const W = CANVAS_W
  const H = CANVAS_H
  const colX = (c: number, n: number) => Math.round((c * W) / n)
  const rowY = (r: number, n: number) => Math.round((r * H) / n)
  const gridRect = (c: number, r: number, n: number): Rect => ({
    x: colX(c, n),
    y: rowY(r, n),
    w: colX(c + 1, n) - colX(c, n),
    h: rowY(r + 1, n) - rowY(r, n),
  })
  switch (layout.id) {
    case 'side2':
      return [
        { x: 0, y: 0, w: colX(1, 2), h: H },
        { x: colX(1, 2), y: 0, w: W - colX(1, 2), h: H },
      ]
    case 'pip': {
      const pw = Math.round(W * 0.3)
      const ph = Math.round(H * 0.3)
      return [
        { x: 0, y: 0, w: W, h: H },
        { x: W - pw - Math.round(W * 0.03), y: H - ph - Math.round(H * 0.04), w: pw, h: ph },
      ]
    }
    case 'quad':
      return [gridRect(0, 0, 2), gridRect(1, 0, 2), gridRect(0, 1, 2), gridRect(1, 1, 2)]
    case 'nine':
      return Array.from({ length: 9 }, (_, i) => gridRect(i % 3, Math.floor(i / 3), 3))
    case 'oneplusfive': {
      const big: Rect = { x: 0, y: 0, w: colX(2, 3), h: rowY(2, 3) }
      const smalls: Array<[number, number]> = [
        [2, 0],
        [2, 1],
        [0, 2],
        [1, 2],
        [2, 2],
      ]
      return [big, ...smalls.map(([c, r]) => gridRect(c, r, 3))]
    }
    case 'single':
    default:
      return [{ x: 0, y: 0, w: W, h: H }]
  }
}

// Server-local RTSP pull URL for a device, derived from its ZLM stream (same
// app/stream as its FLV). The compositor runs on the server, so it always pulls
// from local ZLM regardless of how the browser reaches the dashboard.
function sourceRtsp(d: DeviceItem): string {
  const flv = d.flv_url || `/media/live/${encodeURIComponent(d.id)}.live.flv`
  const m = flv.match(/\/media\/([^/]+)\/(.+)\.live\.flv$/)
  const app = m?.[1] ?? 'live'
  const stream = m?.[2] ?? d.id
  return `rtsp://127.0.0.1:8554/${app}/${stream}`
}

// One region per layout cell (region index === cell index). An empty cell gets
// an empty source so it starts black and can be switched to a source live.
function allCellRegions(): CompositorRegion[] {
  return cellRects(currentLayout.value).map((r, i) => ({
    source: assignments.value[i] ?? '',
    ...r,
  }))
}

// Playback addresses for the composited program, built from the page host + the
// well-known ZLM ports. rtsp/rtmp for VLC/OBS; http-flv (and the same-origin
// /media proxy) for the browser.
const addrs = computed(() => {
  const p = program.value
  if (!p) {
    return null
  }
  const host = window.location.hostname || '127.0.0.1'
  const m = (p.publish_url || '').match(/^\w+:\/\/[^/]+\/(.+)$/)
  const path = m?.[1] ?? `live/${p.id}`
  return {
    rtsp: `rtsp://${host}:8554/${path}`,
    rtmp: `rtmp://${host}:8555/${path}`,
    flv: `http://${host}:8553/${path}.live.flv`,
    proxy: `/media/${path}.live.flv`,
  }
})

const addrRows = computed(() => {
  const a = addrs.value
  if (!a) {
    return []
  }
  return [
    { label: 'RTSP', url: a.rtsp },
    { label: 'RTMP', url: a.rtmp },
    { label: 'HTTP-FLV', url: a.flv },
  ]
})

async function startProgramPlayer(url: string) {
  stopProgramPlayer()
  const video = programVideo.value
  if (!video) {
    return
  }
  try {
    const flvjs = await ensureFlvJs()
    if (!flvjs?.isSupported()) {
      video.src = url
      await video.play().catch(() => {})
      return
    }
    const created = flvjs.createPlayer({ type: 'flv', url, isLive: true })
    programPlayer = created
    created.attachMediaElement(video)
    created.load()
    await created.play().catch(() => {})
  } catch {
    // playback errors surface as a black preview; the addresses stay valid
  }
}

function stopProgramPlayer() {
  const video = programVideo.value
  if (programPlayer) {
    programPlayer.pause?.()
    programPlayer.unload?.()
    programPlayer.detachMediaElement?.()
    programPlayer.destroy()
    programPlayer = null
  }
  if (video) {
    video.pause()
    video.removeAttribute('src')
    video.load()
  }
}

// Create (or re-create) the composited program. The pool is the WHOLE device
// list, so any camera can later be switched into any region live; regions map
// 1:1 to the current layout's cells.
async function goLive() {
  if (!sources.value.length) {
    programError.value = '暂无设备可作为信号源'
    return
  }
  const assignedCount = assignments.value
    .slice(0, currentLayout.value.cells)
    .filter(Boolean).length
  if (!assignedCount) {
    programError.value = '请先给画面格分配信号源'
    return
  }
  programBusy.value = true
  programError.value = ''
  try {
    if (program.value) {
      stopProgramPlayer()
      try {
        await removeCompositor(PROGRAM_ID)
      } catch {
        // already gone — proceed to re-create
      }
      program.value = null
    }
    const created = await createCompositor({
      id: PROGRAM_ID,
      width: CANVAS_W,
      height: CANVAS_H,
      fps: CANVAS_FPS,
      sources: sources.value.map((d) => ({ id: d.id, url: sourceRtsp(d) })),
      regions: allCellRegions(),
    })
    program.value = created
    await nextTick()
    if (addrs.value) {
      await startProgramPlayer(addrs.value.proxy)
    }
  } catch (e) {
    programError.value = e instanceof Error ? e.message : '合成失败'
  } finally {
    programBusy.value = false
  }
}

// Swap one region's source on the running program — no stream restart.
async function liveSwitch(cell: number, sourceId: string) {
  if (!program.value) {
    return
  }
  try {
    await switchRegion(PROGRAM_ID, cell, sourceId)
    programError.value = ''
  } catch (e) {
    programError.value = e instanceof Error ? e.message : '切换失败'
  }
}

// Apply the current layout to the running program live — the server rebuilds
// only its filter graph, so the stream (and the browser preview) never restarts.
async function relayoutProgram() {
  if (!program.value) {
    return
  }
  try {
    await relayoutCompositor(PROGRAM_ID, allCellRegions())
    programError.value = ''
  } catch (e) {
    programError.value = e instanceof Error ? e.message : '布局切换失败'
  }
}

async function stopLive() {
  stopProgramPlayer()
  const had = program.value
  program.value = null
  programError.value = ''
  if (had) {
    try {
      await removeCompositor(PROGRAM_ID)
    } catch {
      // ignore — best-effort teardown
    }
  }
}

// Restore an already-running program (e.g. after navigating back).
async function refreshProgram() {
  try {
    const list = await listCompositors()
    const found = Array.isArray(list) ? (list.find((p) => p.id === PROGRAM_ID) ?? null) : null
    program.value = found
    if (found) {
      await nextTick()
      if (addrs.value) {
        await startProgramPlayer(addrs.value.proxy)
      }
    }
  } catch {
    // no program yet, or list unavailable — leave the panel hidden
  }
}

async function copyAddr(key: string, url: string) {
  try {
    await navigator.clipboard.writeText(url)
  } catch {
    // clipboard blocked (e.g. non-secure context) — user can still select text
  }
  copiedKey.value = key
  if (copyTimer) {
    clearTimeout(copyTimer)
  }
  copyTimer = setTimeout(() => {
    copiedKey.value = ''
  }, 1500)
}

onMounted(async () => {
  await loadSources()
  await refreshProgram()
})

onBeforeUnmount(() => {
  // Keep the program broadcasting server-side; only tear down the local preview.
  stopProgramPlayer()
  if (copyTimer) {
    clearTimeout(copyTimer)
  }
})
</script>

<template>
  <div class="content-section switcher-page">
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">导播台</h1>
        <p class="page-subtitle">多画面监看 · 合成输出(Program)· 无缝切换不断流</p>
      </div>
      <div class="page-actions">
        <Button
          label="重置"
          icon="pi pi-refresh"
          outlined
          size="small"
          :loading="loading"
          @click="loadSources"
        />
      </div>
    </div>

    <div class="switcher-toolbar">
      <span class="toolbar-label">布局</span>
      <div class="layout-list">
        <button
          v-for="l in LAYOUTS"
          :key="l.id"
          type="button"
          class="layout-btn"
          :class="{ 'is-active': l.id === layoutId }"
          @click="setLayout(l.id)"
        >
          <span class="mini" :class="l.cls"><i v-for="k in l.cells" :key="k" /></span>
          {{ l.label }}
        </button>
      </div>
      <span v-if="isLive" class="toolbar-live"><span class="live-dot" />直播中 · 切换无缝不断流</span>
      <div class="program-actions">
        <Button
          v-if="!isLive"
          label="上线合成"
          icon="pi pi-bolt"
          size="small"
          :loading="programBusy"
          @click="goLive"
        />
        <Button
          v-else
          label="停止"
          icon="pi pi-stop-circle"
          severity="danger"
          outlined
          size="small"
          :loading="programBusy"
          @click="stopLive"
        />
      </div>
    </div>

    <div class="switcher-body">
      <aside class="source-panel">
        <div class="source-head">
          <i class="pi pi-video" />
          信号源
          <span class="source-count">{{ sources.length }}</span>
        </div>
        <div v-if="error" class="source-error">{{ error }}</div>
        <div class="source-list">
          <button
            v-for="s in sources"
            :key="s.id"
            type="button"
            class="source-item"
            :class="{ 'is-on': isAssigned(s.id) }"
            @click="assignToActive(s)"
          >
            <span class="source-thumb"><i class="pi pi-video" /></span>
            <span class="source-meta">
              <span class="source-name ellipsis-text">{{ s.name || s.id }}</span>
              <span class="source-id ellipsis-text">{{ s.id }}</span>
            </span>
            <span v-if="isAssigned(s.id)" class="source-on"><i class="pi pi-check" /></span>
            <span v-else class="source-type">{{ s.input_type }}</span>
          </button>
          <div v-if="!sources.length && !loading" class="empty-state source-empty">
            <i class="pi pi-inbox empty-state-icon" />
            <span class="empty-state-text">暂无设备</span>
          </div>
        </div>
      </aside>

      <section class="stage">
        <div class="grid" :class="currentLayout.cls">
          <SwitcherTile
            v-for="i in currentLayout.cells"
            :key="i - 1"
            :index="i - 1"
            :source="sourceById(assignments[i - 1] ?? null)"
            :active="activeCell === i - 1"
            @select="selectCell"
            @enlarge="enlargeCell"
            @clear="clearCell"
          />
        </div>
      </section>
    </div>

    <section v-if="program || programError" class="program-bar data-card">
      <div v-if="program" class="program-preview">
        <video ref="programVideo" class="program-video" muted autoplay playsinline />
        <span class="program-badge"><span class="program-dot" />PROGRAM</span>
      </div>
      <div class="program-info">
        <div class="program-title">
          <i class="pi pi-share-alt" />
          合成输出
          <span v-if="program" class="program-id">{{ program.id }}</span>
          <span v-if="program" class="program-dims">
            {{ program.width }}×{{ program.height }} · {{ program.fps }}fps
          </span>
        </div>
        <p v-if="programError" class="program-error">{{ programError }}</p>
        <ul v-if="addrRows.length" class="addr-list">
          <li v-for="a in addrRows" :key="a.label" class="addr-row">
            <span class="addr-tag">{{ a.label }}</span>
            <code class="addr-url ellipsis-text">{{ a.url }}</code>
            <button
              type="button"
              class="addr-copy"
              :class="{ 'is-copied': copiedKey === a.label }"
              :title="copiedKey === a.label ? '已复制' : '复制地址'"
              @click="copyAddr(a.label, a.url)"
            >
              <i :class="copiedKey === a.label ? 'pi pi-check' : 'pi pi-copy'" />
            </button>
          </li>
        </ul>
        <p class="program-hint">
          <i class="pi pi-info-circle" />
          选中画面格后点左侧信号源即可无缝切换(不断流);用 VLC / OBS 打开上面的 RTSP / RTMP 地址。
        </p>
      </div>
    </section>
  </div>
</template>

<style scoped>
.switcher-page {
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.switcher-toolbar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.6rem 0.9rem;
  padding: 0.6rem 0.85rem;
  border: 1px solid rgb(148 163 184 / 12%);
  border-radius: 0.75rem;
  background: rgb(15 23 42 / 40%);
  backdrop-filter: blur(12px);
}

.toolbar-label {
  font-size: 0.8rem;
  font-weight: 600;
  color: #94a3b8;
}

.layout-list {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
}

.layout-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.45rem;
  padding: 0.35rem 0.6rem;
  border: 1px solid rgb(148 163 184 / 16%);
  border-radius: 0.5rem;
  background: rgb(30 41 59 / 40%);
  color: #cbd5e1;
  font-size: 0.78rem;
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s, color 0.15s;
}

.layout-btn:hover {
  border-color: rgb(59 130 246 / 45%);
  color: #e2e8f0;
}

.layout-btn.is-active {
  border-color: #3b82f6;
  background: rgb(59 130 246 / 18%);
  color: #bfdbfe;
}

.mini {
  display: grid;
  width: 1.05rem;
  height: 1.05rem;
  gap: 1px;
  color: currentcolor;
}

.mini i {
  background: currentcolor;
  border-radius: 1px;
  opacity: 0.85;
}

.mini.single {
  grid-template-columns: 1fr;
  grid-template-rows: 1fr;
}

.mini.side2 {
  grid-template-columns: 1fr 1fr;
  grid-template-rows: 1fr;
}

.mini.quad {
  grid-template-columns: repeat(2, 1fr);
  grid-template-rows: repeat(2, 1fr);
}

.mini.nine {
  grid-template-columns: repeat(3, 1fr);
  grid-template-rows: repeat(3, 1fr);
}

.mini.pip {
  position: relative;
  grid-template-columns: 1fr;
  grid-template-rows: 1fr;
}

.mini.pip i:nth-child(2) {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 45%;
  height: 45%;
}

.mini.oneplusfive {
  grid-template-columns: repeat(3, 1fr);
  grid-template-rows: repeat(3, 1fr);
}

.mini.oneplusfive i:nth-child(1) {
  grid-column: 1 / 3;
  grid-row: 1 / 3;
}

.toolbar-live {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  color: #86efac;
  font-size: 0.74rem;
  font-weight: 600;
}

.live-dot {
  width: 0.45rem;
  height: 0.45rem;
  border-radius: 50%;
  background: #22c55e;
  box-shadow: 0 0 6px rgb(34 197 94 / 70%);
}

.program-actions {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  margin-left: auto;
}

.switcher-body {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 1rem;
}

.source-panel {
  width: 14rem;
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding: 0.75rem;
  border: 1px solid rgb(148 163 184 / 12%);
  border-radius: 0.75rem;
  background: rgb(15 23 42 / 40%);
  backdrop-filter: blur(12px);
  overflow: hidden;
}

.source-head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.85rem;
  font-weight: 600;
  color: #e2e8f0;
}

.source-count {
  padding: 0 0.4rem;
  border-radius: 0.3rem;
  background: rgb(59 130 246 / 18%);
  color: #93c5fd;
  font-size: 0.72rem;
}

.source-error {
  color: #fca5a5;
  font-size: 0.75rem;
}

.source-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.source-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.45rem 0.5rem;
  border: 1px solid rgb(148 163 184 / 12%);
  border-radius: 0.55rem;
  background: rgb(30 41 59 / 40%);
  color: #cbd5e1;
  cursor: pointer;
  text-align: left;
  transition: border-color 0.15s, background 0.15s;
}

.source-item:hover {
  border-color: rgb(59 130 246 / 45%);
  background: rgb(30 41 59 / 65%);
}

.source-item.is-on {
  border-color: rgb(59 130 246 / 55%);
  background: rgb(59 130 246 / 12%);
}

.source-thumb {
  flex: none;
  width: 1.8rem;
  height: 1.8rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 0.4rem;
  background: rgb(15 23 42 / 60%);
  color: #60a5fa;
}

.source-meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.source-name {
  font-size: 0.8rem;
  color: #e2e8f0;
}

.source-id {
  font-family: SFMono-Regular, Consolas, 'Liberation Mono', monospace;
  font-size: 0.68rem;
  color: #64748b;
}

.source-type {
  flex: none;
  padding: 0.1rem 0.35rem;
  border-radius: 0.3rem;
  background: rgb(148 163 184 / 12%);
  color: #94a3b8;
  font-size: 0.65rem;
  text-transform: uppercase;
}

.source-on {
  flex: none;
  color: #3b82f6;
}

.source-empty {
  padding: 1.5rem 0;
}

.stage {
  flex: 1;
  min-height: 0;
  border-radius: 0.75rem;
}

.grid {
  display: grid;
  gap: 6px;
  width: 100%;
  height: 100%;
}

.grid.single {
  grid-template-columns: 1fr;
  grid-template-rows: 1fr;
}

.grid.side2 {
  grid-template-columns: 1fr 1fr;
  grid-template-rows: 1fr;
}

.grid.quad {
  grid-template-columns: repeat(2, 1fr);
  grid-template-rows: repeat(2, 1fr);
}

.grid.nine {
  grid-template-columns: repeat(3, 1fr);
  grid-template-rows: repeat(3, 1fr);
}

.grid.oneplusfive {
  grid-template-columns: repeat(3, 1fr);
  grid-template-rows: repeat(3, 1fr);
}

.grid.oneplusfive :deep(.tile:nth-child(1)) {
  grid-column: 1 / 3;
  grid-row: 1 / 3;
}

.grid.pip {
  position: relative;
  display: block;
}

.grid.pip :deep(.tile:nth-child(1)) {
  position: absolute;
  inset: 0;
}

.grid.pip :deep(.tile:nth-child(2)) {
  position: absolute;
  right: 3%;
  bottom: 4%;
  width: 30%;
  height: 30%;
  z-index: 3;
  box-shadow: 0 8px 20px rgb(0 0 0 / 45%);
}

.program-bar {
  flex: none;
  display: flex;
  gap: 1rem;
  padding: 0.85rem;
  border-radius: 0.75rem;
}

.program-preview {
  position: relative;
  flex: none;
  width: 18rem;
  max-width: 40%;
  aspect-ratio: 16 / 9;
  border-radius: 0.6rem;
  overflow: hidden;
  background: #0b111d;
}

.program-video {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: contain;
  background: #0b111d;
}

.program-badge {
  position: absolute;
  top: 0.4rem;
  left: 0.45rem;
  z-index: 2;
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  padding: 0.1rem 0.4rem;
  border-radius: 0.3rem;
  background: rgb(2 6 15 / 70%);
  color: #f87171;
  font-size: 0.62rem;
  font-weight: 700;
  letter-spacing: 0.05em;
}

.program-dot {
  width: 0.4rem;
  height: 0.4rem;
  border-radius: 50%;
  background: #ef4444;
  box-shadow: 0 0 6px rgb(239 68 68 / 70%);
}

.program-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.program-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.9rem;
  font-weight: 600;
  color: #e2e8f0;
}

.program-id {
  padding: 0.1rem 0.4rem;
  border-radius: 0.3rem;
  background: rgb(59 130 246 / 18%);
  color: #93c5fd;
  font-family: SFMono-Regular, Consolas, 'Liberation Mono', monospace;
  font-size: 0.72rem;
}

.program-dims {
  color: #64748b;
  font-size: 0.72rem;
  font-weight: 400;
}

.program-error {
  margin: 0;
  color: #fca5a5;
  font-size: 0.78rem;
}

.addr-list {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
  margin: 0;
  padding: 0;
  list-style: none;
}

.addr-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.3rem 0.4rem;
  border: 1px solid rgb(148 163 184 / 12%);
  border-radius: 0.45rem;
  background: rgb(30 41 59 / 45%);
}

.addr-tag {
  flex: none;
  width: 4.5rem;
  color: #94a3b8;
  font-size: 0.68rem;
  font-weight: 600;
  letter-spacing: 0.03em;
}

.addr-url {
  flex: 1;
  min-width: 0;
  font-family: SFMono-Regular, Consolas, 'Liberation Mono', monospace;
  font-size: 0.76rem;
  color: #e2e8f0;
}

.addr-copy {
  flex: none;
  width: 1.6rem;
  height: 1.6rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 0.35rem;
  background: rgb(148 163 184 / 12%);
  color: #cbd5e1;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}

.addr-copy:hover {
  background: rgb(59 130 246 / 25%);
  color: #bfdbfe;
}

.addr-copy.is-copied {
  background: rgb(34 197 94 / 22%);
  color: #86efac;
}

.program-hint {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  margin: 0;
  color: #64748b;
  font-size: 0.72rem;
}

@media (width <= 768px) {
  .switcher-body {
    flex-direction: column;
  }

  .source-panel {
    width: 100%;
    max-height: 12rem;
  }

  .program-bar {
    flex-direction: column;
  }

  .program-preview {
    width: 100%;
    max-width: none;
  }
}
</style>
