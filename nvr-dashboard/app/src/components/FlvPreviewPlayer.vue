<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import Button from 'primevue/button'
import Message from 'primevue/message'

type FlvPlayer = {
  attachMediaElement: (element: HTMLVideoElement) => void
  load: () => void
  play: () => Promise<void>
  on?: (event: string, listener: (payload: unknown) => void) => void
  pause?: () => void
  unload?: () => void
  detachMediaElement?: () => void
  destroy: () => void
}

type FlvJs = {
  isSupported: () => boolean
  createPlayer: (mediaDataSource: { type: 'flv'; url: string; isLive: boolean }) => FlvPlayer
}

declare global {
  interface Window {
    flvjs?: FlvJs
  }
}

const props = defineProps<{
  url: string
}>()

const videoRef = ref<HTMLVideoElement | null>(null)
const flvPlayer = ref<FlvPlayer | null>(null)
const previewError = ref('')
const previewMediaInfo = ref<Record<string, unknown>>({})
const previewStats = ref<Record<string, unknown>>({})
const previewInfoVisible = ref(true)
const previewInfoPosition = ref({ x: 20, y: 20 })
const previewInfoDrag = ref<{ startX: number; startY: number; originX: number; originY: number } | null>(null)

watch(
  () => props.url,
  async (url) => {
    stopPreview()
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

  if (!video || !props.url) {
    return
  }

  try {
    const flvjs = await ensureFlvJs()
    if (!flvjs?.isSupported()) {
      video.src = props.url
      await video.play()
      return
    }

    const player = flvjs.createPlayer({
      type: 'flv',
      url: props.url,
      isLive: true,
    })
    player.on?.('media_info', (payload) => {
      previewMediaInfo.value = {
        ...previewMediaInfo.value,
        ...(isRecord(payload) ? payload : {}),
      }
    })
    player.on?.('statistics_info', (payload) => {
      previewStats.value = {
        ...previewStats.value,
        ...(isRecord(payload) ? payload : {}),
      }
    })
    video.addEventListener('loadedmetadata', syncVideoMetadata, { once: true })
    flvPlayer.value = player
    player.attachMediaElement(video)
    player.load()
    await player.play()
  } catch (error) {
    previewError.value = toErrorMessage(error, 'FLV 预览启动失败')
  }
}

function stopPreview() {
  const video = videoRef.value
  if (flvPlayer.value) {
    flvPlayer.value.pause?.()
    flvPlayer.value.unload?.()
    flvPlayer.value.detachMediaElement?.()
    flvPlayer.value.destroy()
    flvPlayer.value = null
  }

  if (video) {
    video.pause()
    video.removeAttribute('src')
    video.load()
  }

  previewMediaInfo.value = {}
  previewStats.value = {}
  previewInfoDrag.value = null
}

async function ensureFlvJs() {
  if (window.flvjs) {
    return window.flvjs
  }

  await new Promise<void>((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>('script[data-flvjs="true"]')
    if (existing) {
      existing.addEventListener('load', () => resolve(), { once: true })
      existing.addEventListener('error', () => reject(new Error('flv.js load failed')), { once: true })
      return
    }

    const script = document.createElement('script')
    script.src = 'https://cdn.jsdelivr.net/npm/flv.js@1.6.2/dist/flv.min.js'
    script.async = true
    script.dataset.flvjs = 'true'
    script.onload = () => resolve()
    script.onerror = () => reject(new Error('flv.js load failed'))
    document.head.appendChild(script)
  })

  return window.flvjs
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
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
      <video
        ref="videoRef"
        class="preview-video"
        controls
        autoplay
        muted
        playsinline
      />
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

.preview-video {
  width: 100%;
  height: min(70vh, 36rem);
  min-height: 18rem;
  max-height: 70vh;
  border-radius: 0.75rem;
  object-fit: contain;
  background:
    radial-gradient(circle at top, rgb(41 98 255 / 15%), transparent 55%),
    linear-gradient(180deg, rgb(10 17 29), rgb(20 26 36));
}

@media (width <= 768px) {
  .preview-info-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .preview-info-panel {
    width: calc(100% - 1rem);
  }

  .preview-video {
    height: min(52vh, 20rem);
    min-height: 14rem;
  }
}
</style>
