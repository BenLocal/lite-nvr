<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import { listDevices, type DeviceItem } from '../api/device'
import SwitcherTile from '../components/SwitcherTile.vue'

interface LayoutDef {
  id: string
  label: string
  cells: number
  cls: string
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

const sources = ref<DeviceItem[]>([])
const loading = ref(false)
const error = ref('')
const layoutId = ref('quad')
const assignments = ref<(string | null)[]>([])
const activeCell = ref(0)
const beforeEnlarge = ref<string | null>(null)

const currentLayout = computed(() => LAYOUTS.find((l) => l.id === layoutId.value) ?? SINGLE_LAYOUT)

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

function setLayout(id: string) {
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
}

function selectCell(index: number) {
  activeCell.value = index
}

function assignToActive(source: DeviceItem) {
  const slots = assignments.value.slice()
  slots[activeCell.value] = source.id
  assignments.value = slots
  // Jump to the next empty slot so several sources can be placed in a row.
  const nextEmpty = slots.findIndex((v, i) => i > activeCell.value && !v)
  if (nextEmpty !== -1) {
    activeCell.value = nextEmpty
  }
}

function clearCell(index: number) {
  const slots = assignments.value.slice()
  slots[index] = null
  assignments.value = slots
  activeCell.value = index
}

// Double-click a tile to blow it up to a single view; double-click again (or
// pick another layout) to return to where you were.
function enlargeCell(index: number) {
  if (layoutId.value === 'single') {
    if (beforeEnlarge.value) {
      const back = beforeEnlarge.value
      setLayout(back)
    }
    return
  }
  const prev = layoutId.value
  const src = assignments.value[index] ?? null
  layoutId.value = 'single'
  assignments.value = [src]
  activeCell.value = 0
  beforeEnlarge.value = prev
}

function isAssigned(id: string): boolean {
  return assignments.value.includes(id)
}

onMounted(loadSources)
</script>

<template>
  <div class="content-section switcher-page">
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">导播台</h1>
        <p class="page-subtitle">多画面监看 · 布局切换 · 信号源切换</p>
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
      <span class="toolbar-hint">
        <i class="pi pi-info-circle" />
        选中画面格(蓝框)后点左侧信号源分配 · 双击画面放大 / 还原
      </span>
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

.toolbar-hint {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  margin-left: auto;
  color: #64748b;
  font-size: 0.72rem;
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

@media (width <= 768px) {
  .switcher-body {
    flex-direction: column;
  }

  .source-panel {
    width: 100%;
    max-height: 12rem;
  }
}
</style>
