<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import Button from 'primevue/button'
import Message from 'primevue/message'
import { createStreamPlayer, type StreamPlayerHandle } from '../utils/streamPlayer'

const props = defineProps<{
  url: string
}>()

const videoRef = ref<HTMLVideoElement | null>(null)
const jessibucaRef = ref<HTMLDivElement | null>(null)
const handle = ref<StreamPlayerHandle | null>(null)
// Bumped on every stop/start so a superseded async start tears itself down.
let gen = 0
const previewError = ref('')
// gb28181 devices pull lazily: the first connection only *triggers* the
// INVITE and is dropped by ZLM (media-not-found hook); the stream registers
// ~1s later. A bounded auto-reconnect turns that mandatory first failure
// (and any transient drop) into a short "connecting" state.
const RETRY_MAX = 5
const RETRY_DELAY_MS = 1500
const retryCount = ref(0)
let retryTimer: ReturnType<typeof setTimeout> | undefined
const previewMediaInfo = ref<Record<string, unknown>>({})
const previewStats = ref<Record<string, unknown>>({})
const previewInfoVisible = ref(true)
const previewInfoPosition = ref({ x: 20, y: 20 })
const previewInfoDrag = ref<{ startX: number; startY: number; originX: number; originY: number } | null>(null)

watch(
  () => props.url,
  async (url) => {
    stopPreview()
    retryCount.value = 0
    previewInfoVisible.value = true
    previewInfoPosition.value = { x: 20, y: 20 }
    if (!url) {
      return
    }
    await nextTick()
    await startPreview()
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  stopPreview()
})

async function startPreview() {
  stopPreview()
  previewError.value = ''
  const video = videoRef.value
  const container = jessibucaRef.value

  if (!video || !container || !props.url) {
    return
  }

  const myGen = ++gen
  // Reliable resolution readout on the mpegts/native path (jessibuca reports it
  // via onMediaInfo instead).
  video.addEventListener('loadedmetadata', syncVideoMetadata, { once: true })
  try {
    const created = await createStreamPlayer(
      { video, container },
      props.url,
      {
        muted: true,
        onMediaInfo: (info) => {
          previewMediaInfo.value = { ...previewMediaInfo.value, ...info }
          retryCount.value = 0
        },
        onStats: (info) => {
          previewStats.value = { ...previewStats.value, ...info }
        },
        onError: (message) => {
          scheduleRetry(message)
        },
      },
    )
    if (myGen !== gen) {
      created.destroy()
      return
    }
    handle.value = created
  } catch (error) {
    scheduleRetry(toErrorMessage(error, 'FLV 预览启动失败'))
  }
}

function scheduleRetry(message: string) {
  if (retryTimer) {
    return // a reconnect is already pending; ignore repeated errors
  }
  if (retryCount.value >= RETRY_MAX) {
    previewError.value = message
    return
  }
  retryCount.value++
  previewError.value = ''
  const myGen = gen
  retryTimer = setTimeout(() => {
    retryTimer = undefined
    if (myGen !== gen) {
      return
    }
    void startPreview()
  }, RETRY_DELAY_MS)
}

function stopPreview() {
  gen++
  if (retryTimer) {
    clearTimeout(retryTimer)
    retryTimer = undefined
  }
  if (handle.value) {
    handle.value.destroy()
    handle.value = null
  }
  videoRef.value?.removeEventListener('loadedmetadata', syncVideoMetadata)

  previewMediaInfo.value = {}
  previewStats.value = {}
  previewInfoDrag.value = null
}

function syncVideoMetadata() {
  const video = videoRef.value
  if (!video) {
    return
  }
  previewMediaInfo.value = {
    ...previewMediaInfo.value,
    width: Number(video.videoWidth) || previewMediaInfo.value.width,
    height: Number(video.videoHeight) || previewMediaInfo.value.height,
  }
}

function toErrorMessage(error: unknown, fallback: string) {
  if (error instanceof Error && error.message) {
    return error.message
  }
  return fallback
}

function formatInfoValue(value: unknown, suffix = '') {
  if (value === null || value === undefined || value === '') {
    return '-'
  }
  return `${value}${suffix}`
}

function previewInfoItems() {
  const media = previewMediaInfo.value
  const stats = previewStats.value
  return [
    { label: '视频编码', value: formatCodecInfo(media.videoCodec, 'video') },
    { label: '音频编码', value: formatCodecInfo(media.audioCodec, 'audio') },
    { label: '分辨率', value: media.width && media.height ? `${media.width} x ${media.height}` : '-' },
    { label: 'FPS', value: formatInfoValue(media.fps ?? stats.fps) },
    { label: '码率', value: formatInfoValue(stats.speed ?? stats.currentSegmentBitrate, ' bps') },
    { label: '音频采样率', value: formatInfoValue(media.audioSampleRate, ' Hz') },
    { label: '声道数', value: formatInfoValue(media.audioChannelCount) },
    { label: '丢帧', value: formatInfoValue(stats.droppedFrames) },
  ]
}

function openPreviewInfo() {
  previewInfoVisible.value = true
}

function closePreviewInfo() {
  previewInfoVisible.value = false
}

function startPreviewInfoDrag(event: PointerEvent) {
  if ((event.target as HTMLElement | null)?.closest('.preview-info-close')) {
    return
  }
  previewInfoDrag.value = {
    startX: event.clientX,
    startY: event.clientY,
    originX: previewInfoPosition.value.x,
    originY: previewInfoPosition.value.y,
  }
  window.addEventListener('pointermove', onPreviewInfoDrag)
  window.addEventListener('pointerup', stopPreviewInfoDrag, { once: true })
}

function onPreviewInfoDrag(event: PointerEvent) {
  if (!previewInfoDrag.value) {
    return
  }
  previewInfoPosition.value = {
    x: Math.max(8, previewInfoDrag.value.originX + event.clientX - previewInfoDrag.value.startX),
    y: Math.max(8, previewInfoDrag.value.originY + event.clientY - previewInfoDrag.value.startY),
  }
}

function stopPreviewInfoDrag() {
  previewInfoDrag.value = null
  window.removeEventListener('pointermove', onPreviewInfoDrag)
}

function formatCodecInfo(value: unknown, kind: 'video' | 'audio') {
  if (typeof value !== 'string' || !value) {
    return '-'
  }

  if (kind === 'video') {
    return formatVideoCodec(value)
  }
  return formatAudioCodec(value)
}

function formatVideoCodec(codec: string) {
  if (codec.startsWith('avc1.')) {
    const profileLevel = codec.slice(5)
    if (profileLevel.length === 6) {
      const profileHex = profileLevel.slice(0, 2).toUpperCase()
      const levelHex = profileLevel.slice(4, 6)
      const profileName =
        {
          '42': 'Baseline',
          '4D': 'Main',
          '58': 'Extended',
          '64': 'High',
          '6E': 'High 10',
          '7A': 'High 4:2:2',
          F4: 'High 4:4:4',
        }[profileHex] ?? 'Unknown'
      const levelNum = Number.parseInt(levelHex, 16)
      const level = Number.isFinite(levelNum) ? `L${(levelNum / 10).toFixed(1)}` : ''
      return `H.264 / ${profileName}${level ? `@${level}` : ''}`
    }
    return 'H.264'
  }

  if (codec.startsWith('hev1.') || codec.startsWith('hvc1.')) {
    return 'H.265 / HEVC'
  }

  if (codec.startsWith('vp8')) {
    return 'VP8'
  }

  if (codec.startsWith('vp9')) {
    return 'VP9'
  }

  if (codec.startsWith('av01')) {
    return 'AV1'
  }

  return codec
}

function formatAudioCodec(codec: string) {
  if (codec.startsWith('mp4a.40.2')) {
    return 'AAC-LC'
  }
  if (codec.startsWith('mp4a.40.5')) {
    return 'HE-AAC'
  }
  if (codec.startsWith('mp4a.40.29')) {
    return 'HE-AACv2'
  }
  if (codec.startsWith('mp4a.40.34')) {
    return 'MP3'
  }
  if (codec.startsWith('opus')) {
    return 'Opus'
  }
  if (codec.startsWith('vorbis')) {
    return 'Vorbis'
  }
  return codec
}
</script>

<template>
  <div class="preview-shell">
    <Button
      icon="pi pi-info-circle"
      text
      rounded
      class="preview-info-toggle"
      aria-label="打开流信息"
      @click="openPreviewInfo"
    />
    <Message v-if="previewError" severity="error" :closable="false">{{ previewError }}</Message>
    <Message v-else-if="retryCount > 0" severity="secondary" :closable="false">
      正在拉流（第 {{ retryCount }}/{{ RETRY_MAX }} 次连接）…
    </Message>
    <div class="preview-stage">
      <div
        v-if="previewInfoVisible"
        class="preview-info-panel"
        :style="{ transform: `translate(${previewInfoPosition.x}px, ${previewInfoPosition.y}px)` }"
      >
        <div class="preview-info-panel-header" @pointerdown="startPreviewInfoDrag">
          <div class="preview-info-panel-title">流信息</div>
          <Button
            icon="pi pi-times"
            text
            rounded
            class="preview-info-close"
            aria-label="关闭流信息"
            @click="closePreviewInfo"
          />
        </div>
        <div class="preview-info-grid">
          <div v-for="item in previewInfoItems()" :key="item.label" class="preview-info-card">
            <div class="preview-info-label">{{ item.label }}</div>
            <div class="preview-info-value">{{ item.value }}</div>
          </div>
        </div>
      </div>
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
    </div>
  </div>
</template>

<style scoped>
.preview-shell {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  overflow: hidden;
  position: relative;
}

.preview-info-toggle {
  position: absolute;
  top: 0.25rem;
  right: 0.25rem;
  z-index: 3;
  background: transparent;
  color: white;
}

.preview-stage {
  position: relative;
  overflow: hidden;
}

.preview-info-panel {
  position: absolute;
  left: 0;
  top: 0;
  z-index: 2;
  width: min(32rem, calc(100% - 1rem));
  max-width: calc(100% - 1rem);
  padding: 0.75rem;
  border: 1px solid rgb(255 255 255 / 24%);
  border-radius: 0.9rem;
  background: transparent;
  box-shadow: none;
  backdrop-filter: none;
}

.preview-info-panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.75rem;
  cursor: move;
  user-select: none;
}

.preview-info-panel-title {
  font-size: 0.9rem;
  font-weight: 700;
  color: rgb(255 255 255 / 92%);
}

.preview-info-close {
  color: white;
}

.preview-info-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.6rem;
}

.preview-info-card {
  min-width: 0;
  padding: 0.65rem 0.75rem;
  border: 1px solid rgb(255 255 255 / 16%);
  border-radius: 0.75rem;
  background: transparent;
}

.preview-info-label {
  font-size: 0.75rem;
  color: rgb(255 255 255 / 58%);
}

.preview-info-value {
  margin-top: 0.35rem;
  font-family: SFMono-Regular, Consolas, 'Liberation Mono', monospace;
  font-size: 0.9rem;
  font-weight: 600;
  color: rgb(255 255 255 / 92%);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.preview-media {
  position: relative;
  width: 100%;
  height: min(56vh, 32rem);
  min-height: 16rem;
  max-height: 56vh;
}

.preview-video {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  border-radius: 0.75rem;
  object-fit: contain;
  background:
    radial-gradient(circle at top, rgb(41 98 255 / 15%), transparent 55%),
    linear-gradient(180deg, rgb(10 17 29), rgb(20 26 36));
}

/* Jessibuca mounts its canvas here; shown only when it is the active backend. */
.preview-jessibuca {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  display: none;
  overflow: hidden;
  border-radius: 0.75rem;
  background: #0b111d;
}

@media (width <= 768px) {
  .preview-info-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .preview-info-panel {
    width: calc(100% - 1rem);
  }

  .preview-media {
    height: min(52vh, 20rem);
    min-height: 14rem;
  }
}
</style>
