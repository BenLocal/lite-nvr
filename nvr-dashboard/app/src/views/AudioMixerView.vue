<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import Card from 'primevue/card'
import Button from 'primevue/button'
import Slider from 'primevue/slider'
import ToggleSwitch from 'primevue/toggleswitch'
import Select from 'primevue/select'
import Dialog from 'primevue/dialog'
import InputText from 'primevue/inputtext'
import Checkbox from 'primevue/checkbox'
import { useAppToast } from '../utils/toast'
import { listDevices, type DeviceItem } from '../api/device'
import {
  addBusInput,
  createBus,
  getMixer,
  removeBus,
  removeBusInput,
  setBusInputMute,
  setBusInputVolume,
  type MixerBus,
  type MixerState,
} from '../api/audiomixer'
import { createStreamPlayer, type StreamPlayerHandle } from '../utils/streamPlayer'

const appToast = useAppToast()

const devices = ref<DeviceItem[]>([])
const state = ref<MixerState>({ sources: [], buses: [] })
const loading = ref(true)

/** Devices that carry audio — the only valid mixer inputs. */
const audioDevices = computed(() => devices.value.filter((d) => d.include_audio))

function deviceName(id: string): string {
  return devices.value.find((d) => d.id === id)?.name ?? id
}

/** Audio devices not yet mixed into `bus` (candidates for its add-input select). */
function availableFor(bus: MixerBus) {
  const used = new Set(bus.inputs.map((i) => i.source_id))
  return audioDevices.value
    .filter((d) => !used.has(d.id))
    .map((d) => ({ label: d.name, value: d.id }))
}

async function refresh() {
  try {
    state.value = await getMixer()
  } catch (error) {
    appToast.errorFrom('加载失败', error, '无法获取混音台状态')
  }
}

onMounted(async () => {
  try {
    const [devs] = await Promise.all([listDevices(), refresh()])
    devices.value = devs
  } catch (error) {
    appToast.errorFrom('加载失败', error, '无法获取设备列表')
  } finally {
    loading.value = false
  }
})

// ---- create bus dialog ----------------------------------------------------
const showCreate = ref(false)
const newBusId = ref('')
const newBusInputs = ref<string[]>([])
const creating = ref(false)

function openCreate() {
  newBusId.value = ''
  newBusInputs.value = []
  showCreate.value = true
}

async function onCreateBus() {
  const id = newBusId.value.trim()
  if (!id) {
    appToast.error('无法创建', '请填写总线名称')
    return
  }
  if (newBusInputs.value.length === 0) {
    appToast.error('无法创建', '至少选择一路输入')
    return
  }
  creating.value = true
  try {
    state.value = await createBus({
      id,
      inputs: newBusInputs.value.map((source_id) => ({ source_id })),
    })
    showCreate.value = false
    appToast.success('已创建', `输出总线「${id}」已开始推流`)
  } catch (error) {
    appToast.errorFrom('创建失败', error, '无法创建输出总线')
  } finally {
    creating.value = false
  }
}

async function onRemoveBus(bus: MixerBus) {
  try {
    if (listeningId.value === bus.id) await stopListen()
    await removeBus(bus.id)
    await refresh()
  } catch (error) {
    appToast.errorFrom('删除失败', error, '无法删除输出总线')
  }
}

// ---- per-input controls ---------------------------------------------------
const addSelection = ref<Record<string, string | null>>({})

async function onAddInput(bus: MixerBus) {
  const sourceId = addSelection.value[bus.id]
  if (!sourceId) return
  try {
    await addBusInput(bus.id, sourceId)
    addSelection.value[bus.id] = null
    await refresh()
  } catch (error) {
    appToast.errorFrom('添加失败', error, '无法添加输入')
  }
}

async function onRemoveInput(bus: MixerBus, sourceId: string) {
  try {
    await removeBusInput(bus.id, sourceId)
    await refresh()
  } catch (error) {
    appToast.errorFrom('移除失败', error, '无法移除输入')
  }
}

async function onVolume(busId: string, sourceId: string, volume: number) {
  try {
    await setBusInputVolume(busId, sourceId, volume)
  } catch (error) {
    appToast.errorFrom('调整失败', error, '无法设置音量')
  }
}

async function onMute(busId: string, sourceId: string, muted: boolean) {
  try {
    await setBusInputMute(busId, sourceId, muted)
  } catch (error) {
    appToast.errorFrom('操作失败', error, '无法设置静音')
  }
}

// ---- monitor / listen -----------------------------------------------------
const listeningId = ref<string | null>(null)
const monitorVideo = ref<HTMLVideoElement | null>(null)
const monitorContainer = ref<HTMLElement | null>(null)
const waveCanvas = ref<HTMLCanvasElement | null>(null)
let monitorHandle: StreamPlayerHandle | null = null

// Web Audio taps the monitor <video> to draw a live waveform. A
// MediaElementSource can be created only once per element, so the context /
// source / analyser are built lazily and reused across listen sessions.
let audioCtx: AudioContext | null = null
let sourceNode: MediaElementAudioSourceNode | null = null
let analyser: AnalyserNode | null = null
let waveData: Uint8Array<ArrayBuffer> | null = null
let rafId = 0
// Current volume level 0–100 (VU meter beside the waveform), with a decaying
// peak hold so it doesn't flicker.
const meterLevel = ref(0)
let meterSmooth = 0

function startWaveform() {
  const video = monitorVideo.value
  if (!video || !waveCanvas.value) return
  try {
    if (!audioCtx) audioCtx = new AudioContext()
    void audioCtx.resume()
    if (!sourceNode) {
      sourceNode = audioCtx.createMediaElementSource(video)
      analyser = audioCtx.createAnalyser()
      analyser.fftSize = 1024
      // source -> analyser -> speakers (must reach destination or it goes mute).
      sourceNode.connect(analyser)
      analyser.connect(audioCtx.destination)
      waveData = new Uint8Array(analyser.frequencyBinCount)
    }
    drawWaveform()
  } catch {
    // Best-effort (e.g. a non-<video> player backend); ignore failures.
  }
}

function drawWaveform() {
  const canvas = waveCanvas.value
  if (!canvas || !analyser || !waveData) return
  analyser.getByteTimeDomainData(waveData)

  // Volume level (0–100) from RMS; full-scale sine (~0.707 RMS) maps to ~100.
  // Fast attack, slow decay for a readable VU-style meter.
  let sumSq = 0
  for (let i = 0; i < waveData.length; i++) {
    const d = ((waveData[i] ?? 128) - 128) / 128
    sumSq += d * d
  }
  const level = Math.min(100, Math.round(Math.sqrt(sumSq / waveData.length) * 141))
  meterSmooth = level > meterSmooth ? level : meterSmooth * 0.9
  meterLevel.value = Math.round(meterSmooth)

  const ctx = canvas.getContext('2d')
  if (ctx) {
    const { width, height } = canvas
    ctx.clearRect(0, 0, width, height)
    ctx.lineWidth = 2
    ctx.strokeStyle = '#38bdf8'
    ctx.beginPath()
    const step = width / waveData.length
    for (let i = 0; i < waveData.length; i++) {
      const y = ((waveData[i] ?? 128) / 128) * (height / 2) // 128 = centre
      const x = i * step
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    }
    ctx.stroke()
  }
  rafId = requestAnimationFrame(drawWaveform)
}

function stopWaveform() {
  if (rafId) {
    cancelAnimationFrame(rafId)
    rafId = 0
  }
  meterLevel.value = 0
  meterSmooth = 0
  const canvas = waveCanvas.value
  const ctx = canvas?.getContext('2d')
  if (canvas && ctx) ctx.clearRect(0, 0, canvas.width, canvas.height)
}

async function stopListen() {
  stopWaveform()
  monitorHandle?.destroy()
  monitorHandle = null
  listeningId.value = null
}

async function toggleListen(bus: MixerBus) {
  if (listeningId.value === bus.id) {
    await stopListen()
    return
  }
  await stopListen()
  const video = monitorVideo.value
  const container = monitorContainer.value
  if (!video || !container) return
  listeningId.value = bus.id
  try {
    monitorHandle = await createStreamPlayer({ video, container }, bus.flv_url, {
      muted: false,
      onError: (message) => appToast.error('试听失败', message),
    })
    startWaveform()
  } catch (error) {
    listeningId.value = null
    appToast.errorFrom('试听失败', error, '无法播放混音输出')
  }
}

onBeforeUnmount(() => {
  void stopListen()
  void audioCtx?.close()
})
</script>

<template>
  <div class="content-section">
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">混音台</h1>
        <p class="page-subtitle">把多路设备音频混成独立的输出总线，实时增删输入、调音量、静音</p>
      </div>
      <div class="page-actions">
        <Button label="新建输出总线" icon="pi pi-plus" size="small" @click="openCreate" />
      </div>
    </div>

    <!-- Single monitor player: audio-only, one bus at a time. -->
    <div class="monitor" :class="{ 'monitor-active': listeningId }">
      <video ref="monitorVideo" class="monitor-video" autoplay playsinline />
      <!-- Separate mount point for the Jessibuca backend; the stream player
           toggles its display, so it must NOT be the bar wrapping the waveform. -->
      <div ref="monitorContainer" class="monitor-jbc" />
      <span v-if="listeningId" class="monitor-label">
        <i class="pi pi-volume-up" /> 正在试听：{{ listeningId }}
      </span>
      <canvas v-show="listeningId" ref="waveCanvas" class="wave" width="360" height="44" />
      <div v-show="listeningId" class="meter" title="音量 0–100">
        <span class="meter-end">0</span>
        <div class="meter-track">
          <div class="meter-cover" :style="{ width: 100 - meterLevel + '%' }" />
        </div>
        <span class="meter-end">100</span>
        <span class="meter-value">{{ meterLevel }}</span>
      </div>
      <Button
        v-if="listeningId"
        label="停止"
        icon="pi pi-stop"
        severity="secondary"
        size="small"
        text
        @click="stopListen"
      />
    </div>

    <div v-if="!loading && state.buses.length === 0" class="empty-hint">
      还没有输出总线。点击「新建输出总线」，选择几路设备音频开始混音。
    </div>

    <div class="bus-grid">
      <Card v-for="bus in state.buses" :key="bus.id" class="data-card bus-card">
        <template #header>
          <div class="bus-header">
            <div class="bus-title">
              <i class="pi pi-sliders-h bus-icon" />
              <span>{{ bus.id }}</span>
            </div>
            <div class="bus-actions">
              <Button
                :label="listeningId === bus.id ? '停止' : '试听'"
                :icon="listeningId === bus.id ? 'pi pi-stop' : 'pi pi-headphones'"
                size="small"
                text
                @click="toggleListen(bus)"
              />
              <Button
                icon="pi pi-trash"
                severity="danger"
                size="small"
                text
                @click="onRemoveBus(bus)"
              />
            </div>
          </div>
        </template>
        <template #content>
          <p class="bus-publish">{{ bus.publish_url }}</p>

          <div v-if="bus.inputs.length === 0" class="bus-empty">未混入任何输入</div>

          <div v-for="input in bus.inputs" :key="input.source_id" class="input-row">
            <div class="input-head">
              <span class="input-name" :class="{ muted: input.muted }">
                {{ deviceName(input.source_id) }}
              </span>
              <div class="input-head-right">
                <ToggleSwitch
                  :model-value="!input.muted"
                  @update:model-value="onMute(bus.id, input.source_id, !$event)"
                />
                <Button
                  icon="pi pi-times"
                  severity="secondary"
                  size="small"
                  text
                  @click="onRemoveInput(bus, input.source_id)"
                />
              </div>
            </div>
            <div class="input-fader">
              <Slider
                v-model="input.volume"
                :min="0"
                :max="200"
                class="fader"
                :disabled="input.muted"
                @change="onVolume(bus.id, input.source_id, input.volume)"
              />
              <span class="input-vol">{{ input.volume }}%</span>
            </div>
          </div>

          <div class="add-input">
            <Select
              v-model="addSelection[bus.id]"
              :options="availableFor(bus)"
              option-label="label"
              option-value="value"
              placeholder="添加输入…"
              size="small"
              class="add-select"
              :disabled="availableFor(bus).length === 0"
            />
            <Button
              label="添加"
              icon="pi pi-plus"
              size="small"
              :disabled="!addSelection[bus.id]"
              @click="onAddInput(bus)"
            />
          </div>
        </template>
      </Card>
    </div>

    <Dialog v-model:visible="showCreate" modal header="新建输出总线" :style="{ width: '28rem' }">
      <div class="field">
        <label for="bus-id">总线名称</label>
        <InputText id="bus-id" v-model="newBusId" placeholder="例如 hall / stream" class="field-input" />
        <p class="field-hint">推流地址为 rtmp://…/mixer/{名称}，也是试听用的流名。</p>
      </div>
      <div class="field">
        <label>选择输入（设备音频，至少一路）</label>
        <div v-if="audioDevices.length === 0" class="field-hint">没有带音频的设备。</div>
        <div v-for="d in audioDevices" :key="d.id" class="pick-row">
          <Checkbox v-model="newBusInputs" :value="d.id" :input-id="'nb-' + d.id" />
          <label :for="'nb-' + d.id">{{ d.name }}</label>
        </div>
      </div>
      <template #footer>
        <Button label="取消" severity="secondary" text @click="showCreate = false" />
        <Button label="创建" icon="pi pi-check" :loading="creating" @click="onCreateBus" />
      </template>
    </Dialog>
  </div>
</template>

<style scoped>
.monitor {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-height: 0;
  margin-bottom: 1rem;
}

.monitor-active {
  padding: 0.5rem 0.75rem;
  background: rgb(56 189 248 / 10%);
  border: 1px solid rgb(56 189 248 / 35%);
  border-radius: 0.5rem;
}

.monitor-video,
.monitor-jbc {
  width: 0;
  height: 0;
}

.monitor-label {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.85rem;
  color: #7dd3fc;
}

.wave {
  width: 360px;
  height: 44px;
  max-width: 100%;
  background: rgb(15 23 42 / 60%);
  border-radius: 0.375rem;
}

.meter {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  height: 44px;
  font-size: 0.65rem;
  color: #64748b;
  font-variant-numeric: tabular-nums;
}

.meter-end {
  line-height: 1;
}

.meter-track {
  position: relative;
  width: 180px;
  max-width: 40vw;
  height: 12px;
  border-radius: 3px;
  overflow: hidden;

  /* green (left) -> yellow -> red (right); filled up to the current level */
  background: linear-gradient(to right, #22c55e, #eab308 55%, #ef4444);
}

.meter-cover {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  background: rgb(15 23 42 / 85%);
  transition: width 0.05s linear;
}

.meter-value {
  min-width: 1.6rem;
  font-size: 0.8rem;
  color: #94a3b8;
}

.empty-hint,
.bus-empty {
  color: #94a3b8;
  font-size: 0.85rem;
}

.empty-hint {
  padding: 2rem 0;
  text-align: center;
}

.bus-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(20rem, 1fr));
  gap: 1rem;
}

.bus-card {
  max-width: 100%;
}

.bus-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.25rem 0;
}

.bus-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-weight: 600;
  color: #e2e8f0;
}

.bus-icon {
  color: #38bdf8;
}

.bus-actions {
  display: flex;
  align-items: center;
  gap: 0.25rem;
}

.bus-publish {
  margin: 0 0 0.75rem;
  font-size: 0.75rem;
  color: #64748b;
  word-break: break-all;
}

.input-row {
  padding: 0.6rem 0;
  border-top: 1px solid rgb(148 163 184 / 15%);
}

.input-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.4rem;
}

.input-head-right {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.input-name {
  font-size: 0.9rem;
  color: #e2e8f0;
}

.input-name.muted {
  color: #64748b;
  text-decoration: line-through;
}

.input-fader {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.fader {
  flex: 1;
}

.input-vol {
  width: 3rem;
  text-align: right;
  font-size: 0.8rem;
  color: #94a3b8;
  font-variant-numeric: tabular-nums;
}

.add-input {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.75rem;
  padding-top: 0.75rem;
  border-top: 1px solid rgb(148 163 184 / 15%);
}

.add-select {
  flex: 1;
}

.field {
  margin-bottom: 1rem;
}

.field label {
  display: block;
  margin-bottom: 0.4rem;
  font-size: 0.85rem;
  color: #cbd5e1;
}

.field-input {
  width: 100%;
}

.field-hint {
  margin: 0.4rem 0 0;
  font-size: 0.78rem;
  color: #94a3b8;
}

.pick-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.25rem 0;
}

.pick-row label {
  margin: 0;
  color: #e2e8f0;
  font-size: 0.9rem;
}
</style>
