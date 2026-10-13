<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Card from 'primevue/card'
import Button from 'primevue/button'
import Slider from 'primevue/slider'
import ToggleSwitch from 'primevue/toggleswitch'
import Select from 'primevue/select'
import Checkbox from 'primevue/checkbox'
import InputNumber from 'primevue/inputnumber'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { useAppToast } from '../utils/toast'
import {
  startBus, stopBus,
  getMixer,
  setBusInputMute,
  setBusInputVolume,
  type MixerBus,
  type MixerState,
  getAudioSources, setBusInputChannels,
  type AudioProfile, type AudioSourceOption, type ChannelMap,
} from '../api/audiomixer'
import { sampleRateOptions } from '../forms/outputForm'
import { createStreamPlayer, type StreamPlayerHandle } from '../utils/streamPlayer'

const appToast = useAppToast()

const state = ref<MixerState>({ sources: [], buses: [] })
const loading = ref(true)
const sources = ref<AudioSourceOption[]>([])
const physicalRows = computed(() => sources.value.filter(source => source.kind === 'alsa').map(source => {
  const input = state.value.sources.find(item => item.id === source.id)
  return {
    id: source.id,
    name: source.name,
    channels: '左右双声道',
    inputState: input?.ready === true ? '已收到音频' : input?.ready === false ? '等待信号' : '未接入',
    error: input?.error ?? '',
    busId: state.value.buses[0]?.id ?? '',
    busInput: state.value.buses[0]?.inputs.find(item => item.source_id === source.id),
  }
}))
const codecOptions = [{ label: 'AAC', value: 'aac' }, { label: 'Opus', value: 'opus' }]
const channelOptions = [{ label: '左右独立', value: 'stereo' }, { label: '交换左右', value: 'swap' }, { label: '左声道到两路', value: 'left_to_both' }, { label: '右声道到两路', value: 'right_to_both' }]
const statusAbort = new AbortController()
let statusTimer: ReturnType<typeof setInterval> | undefined
let loadingStatus = false
async function loadStatus(reset = false): Promise<void> {
  if (loadingStatus || statusAbort.signal.aborted) return
  loadingStatus = true
  try {
    const mixer = await getMixer(statusAbort.signal)
    if (!statusAbort.signal.aborted) {
      state.value = mixer
      if (reset) {
        newBusProfile.value = { ...(mixer.buses[0]?.profile ?? defaultProfile()) }
        selectedSourceIds.value = mixer.buses[0]?.inputs.map(input => input.source_id) ?? []
      }
    }
  } finally { loadingStatus = false }
}

async function onChannels(busId: string, sourceId: string, value: ChannelMap): Promise<void> {
  try { await setBusInputChannels(busId, sourceId, value) }
  catch (error) { appToast.errorFrom('设置失败', error, '无法修改声道映射'); await refresh() }
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
    const [available] = await Promise.all([getAudioSources(), loadStatus(true)])
    sources.value = available
    selectedSourceIds.value = selectedSourceIds.value.filter(id => available.some(source => source.kind === 'alsa' && source.id === id))
    if (!statusAbort.signal.aborted) statusTimer = setInterval(() => { void loadStatus().catch(error => { if (!statusAbort.signal.aborted) appToast.errorFrom('状态更新失败', error, '无法读取混音台状态') }) }, 2000)
  } catch (error) {
    appToast.errorFrom('加载失败', error, '无法获取设备列表')
  } finally {
    loading.value = false
  }
})

onBeforeUnmount(() => { statusAbort.abort(); if (statusTimer) clearInterval(statusTimer) })

// ---- one output bus -------------------------------------------------------
const selectedSourceIds = ref<string[]>([])
const changingBus = ref(false)
const running = computed(() => state.value.buses[0]?.enabled === true)
function defaultProfile(): AudioProfile {
  return { codec: 'aac', sample_rate: 48000, channels: 2, bitrate: 128000 }
}
const newBusProfile = ref<AudioProfile>(defaultProfile())
const busRates = computed(() => sampleRateOptions(newBusProfile.value.codec).filter(option => option.value !== null))
watch(() => newBusProfile.value.codec, () => {
  if (!busRates.value.some(option => option.value === newBusProfile.value.sample_rate)) newBusProfile.value.sample_rate = 48000
})
async function toggleBus(): Promise<void> {
  const bus = state.value.buses[0]
  if (!running.value && selectedSourceIds.value.length === 0) {
    appToast.error('无法开始', '至少选择一路硬件音频输入')
    return
  }
  changingBus.value = true
  try {
    if (bus?.enabled) {
      await stopListen()
      state.value = await stopBus(bus.id)
      appToast.success('已停止', '混音总线已停止输出')
    } else {
      state.value = await startBus({
        id: bus?.id ?? 'main', profile: newBusProfile.value,
        inputs: selectedSourceIds.value.map(source_id => ({ source_id, volume: bus?.inputs.find(input => input.source_id === source_id)?.volume ?? 100 })),
      })
      appToast.success('已开始', '混音总线已开始输出')
    }
  } catch (error) {
    appToast.errorFrom('操作失败', error, '无法切换混音总线')
    await refresh()
  } finally { changingBus.value = false }
}

// ---- per-input controls ---------------------------------------------------
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
        <p class="page-subtitle">将硬件音频输入混合到一条输出总线；输入音量、静音和声道映射实时生效。</p>
      </div>
      <div class="page-actions">
        <Button :label="running ? '停止' : '开始'" :icon="running ? 'pi pi-stop' : 'pi pi-play'" :severity="running ? 'danger' : undefined" size="small" :loading="changingBus" :disabled="loading" @click="toggleBus" />
        <Button :label="listeningId ? '停止试听' : '试听'" :icon="listeningId ? 'pi pi-stop' : 'pi pi-headphones'" severity="secondary" size="small" :disabled="!running || changingBus" @click="state.buses[0] && toggleListen(state.buses[0])" />
      </div>
    </div>

    <!-- Single monitor player: audio-only, one bus at a time. -->
    <div class="monitor" :class="{ 'monitor-active': listeningId }">
      <video ref="monitorVideo" class="monitor-video" autoplay playsinline />
      <!-- Separate mount point for the Jessibuca backend; the stream player
           toggles its display, so it must NOT be the bar wrapping the waveform. -->
      <div ref="monitorContainer" class="monitor-jbc" />
      <span v-if="listeningId" class="monitor-label">
        <i class="pi pi-volume-up" /> 正在试听混音
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
    </div>

    <Card class="data-card physical-input-card">
      <template #header>
        <div class="card-header">
          <i class="pi pi-video card-header-icon" />
          <span class="card-header-title">音频输入节点（{{ physicalRows.length }}）</span>
        </div>
      </template>
      <template #content>
        <DataTable :value="physicalRows" data-key="id" :loading="loading" size="small" scrollable>
          <template #empty>未发现端子或 HDMI 音频采集节点</template>
          <Column header="选择">
            <template #body="{ data }"><Checkbox v-model="selectedSourceIds" :value="data.id" :input-id="'input-' + data.id" :aria-label="'选择' + data.name" :disabled="running || changingBus" /></template>
          </Column>
          <Column field="name" header="输入节点" />
          <Column field="id" header="采集设备" />
          <Column field="channels" header="声道" />
          <Column field="inputState" header="输入状态" />
          <Column header="音量">
            <template #body="{ data }">
              <div v-if="data.busInput" class="input-fader">
                <Slider v-model="data.busInput.volume" :min="0" :max="200" class="fader" :disabled="!running || data.busInput.muted" :aria-label="data.name + '音量'" @change="onVolume(data.busId, data.id, data.busInput.volume)" />
                <span class="input-vol">{{ data.busInput.volume }}%</span>
              </div>
              <span v-else>—</span>
            </template>
          </Column>
          <Column header="声音">
            <template #body="{ data }"><ToggleSwitch v-if="data.busInput" :model-value="!data.busInput.muted" :disabled="!running" :aria-label="data.name + '声音'" @update:model-value="onMute(data.busId, data.id, !$event)" /></template>
          </Column>
          <Column header="声道映射">
            <template #body="{ data }"><Select v-if="data.busInput" :model-value="data.busInput.channel_map ?? 'stereo'" :options="channelOptions" option-label="label" option-value="value" :disabled="!running" :aria-label="data.name + '声道映射'" @update:model-value="onChannels(data.busId, data.id, $event)" /></template>
          </Column>
          <Column field="error" header="提示" />
        </DataTable>
        <div class="audio-profile-fields">
          <div class="field"><label for="bus-codec">音频编码</label><Select input-id="bus-codec" v-model="newBusProfile.codec" :options="codecOptions" option-label="label" option-value="value" class="field-input" :disabled="running || changingBus" /></div>
          <div class="field"><label for="bus-rate">节目采样率</label><Select input-id="bus-rate" v-model="newBusProfile.sample_rate" :options="busRates" option-label="label" option-value="value" class="field-input" :disabled="running || changingBus" /></div>
          <div class="field"><label for="bus-bitrate">音频码率</label><InputNumber input-id="bus-bitrate" v-model="newBusProfile.bitrate" :min="16000" :max="512000" :step="1000" suffix=" bps" :use-grouping="false" class="field-input" :disabled="running || changingBus" /></div>
        </div>
        <span class="field-hint">{{ running ? '停止后可修改输入和输出音频参数。' : '点击“开始”保存设置并启动默认混音总线。' }}</span>
      </template>
    </Card>


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

.input-fader {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-width: 160px;
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

.physical-input-card {
  margin-bottom: 1rem;
}

.audio-profile-fields {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 1rem;
  margin-top: 1rem;
}

@media (width <= 768px) {
  .audio-profile-fields { grid-template-columns: 1fr; }
}
</style>
