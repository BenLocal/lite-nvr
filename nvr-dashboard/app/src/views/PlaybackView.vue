<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Button from 'primevue/button'
import Card from 'primevue/card'
import Dialog from 'primevue/dialog'
import Paginator from 'primevue/paginator'
import Tag from 'primevue/tag'
import Tab from 'primevue/tab'
import TabList from 'primevue/tablist'
import TabPanel from 'primevue/tabpanel'
import TabPanels from 'primevue/tabpanels'
import Tabs from 'primevue/tabs'
import {
  buildPlaybackPlaylistUrl,
  buildPlaybackSegmentPlaylistUrl,
  listDevicePlaybackSegments,
  listTodayDevicePlaybackSegments,
  listPlayback,
  type DevicePlaybackItem,
  type PlaybackSegmentItem,
} from '../api/playback'
import { useAppToast } from '../utils/toast'

const DAY_SECONDS = 24 * 60 * 60
const TIMELINE_TICKS = [0, 6, 12, 18, 24]

type PlaybackMode = 'segment' | 'playlist'
type HlsPlayer = {
  loadSource: (url: string) => void
  attachMedia: (element: HTMLVideoElement) => void
  destroy: () => void
}
type HlsJs = {
  isSupported: () => boolean
  new (): HlsPlayer
}

declare global {
  interface Window {
    Hls?: HlsJs
  }
}

const appToast = useAppToast()

const loading = ref(false)
const devices = ref<DevicePlaybackItem[]>([])
const activeDeviceId = ref('')
const playerRef = ref<HTMLVideoElement | null>(null)
const timelineTrackRef = ref<HTMLDivElement | null>(null)
const hlsPlayerRef = ref<HlsPlayer | null>(null)
const currentSegmentId = ref('')
const currentPlayerUrl = ref('')
const currentPlaybackSecond = ref(0)
const timelineSecond = ref(0)
const pendingSeekSecond = ref<number | null>(null)
const isDraggingTimeline = ref(false)
const detailVisible = ref(false)
const detailSegment = ref<PlaybackSegmentItem | null>(null)
const pageFirst = ref(0)
const pageRows = ref(8)
const playbackVisible = ref(false)
const playbackMode = ref<PlaybackMode>('segment')
const devicePage = ref(1)
const devicePageSize = ref(12)
const totalDevices = ref(0)
const segmentLoading = ref(false)
const deviceSegmentsMap = ref<Record<string, PlaybackSegmentItem[]>>({})
const todayDeviceSegmentsMap = ref<Record<string, PlaybackSegmentItem[]>>({})
const activeDeviceSegmentsTotal = ref(0)

const totalSegments = computed(() =>
  devices.value.reduce((total, item) => total + item.segment_count, 0),
)

const activeDevice = computed(() =>
  devices.value.find((item) => item.device_id === activeDeviceId.value) ?? null,
)

const activeDeviceSegments = computed(() => deviceSegmentsMap.value[activeDeviceId.value] ?? [])
const activeTodaySegments = computed(() => todayDeviceSegmentsMap.value[activeDeviceId.value] ?? [])

const todayRange = computed(() => {
  const now = new Date()
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const end = new Date(start)
  end.setDate(start.getDate() + 1)
  return {
    startMs: start.getTime(),
    endMs: end.getTime(),
  }
})

const todaySegments = computed(() => {
  const device = activeDevice.value
  if (!device) {
    return []
  }
  return [...activeTodaySegments.value]
    .filter((segment) => {
      const startMs = segment.start_time * 1000
      const endMs = startMs + segment.duration * 1000
      return endMs > todayRange.value.startMs && startMs < todayRange.value.endMs
    })
    .sort((a, b) => a.start_time - b.start_time)
})

const currentSegment = computed(() =>
  todaySegments.value.find((segment) => segment.id === currentSegmentId.value) ?? null,
)

const activeSegments = computed(() => {
  if (!activeDevice.value) {
    return []
  }
  return [...activeDeviceSegments.value].sort((a, b) => b.start_time - a.start_time)
})

const pagedSegments = computed(() => activeSegments.value)

const currentTimelineLabel = computed(() => formatDaySecond(timelineSecond.value))

const currentSegmentElapsed = computed(() => {
  if (!currentSegment.value) {
    return '未播放'
  }
  return `${formatDuration(currentPlayerTimeInSegment())} / ${formatDuration(currentSegment.value.duration)}`
})

const timelineBackground = computed(() => buildTimelineBackground(todaySegments.value))

onMounted(() => {
  void loadPlayback()
})

onBeforeUnmount(() => {
  stopPlayer()
  stopTimelineDrag()
})

watch(activeDeviceId, () => {
  resetPlayback(false)
  pageFirst.value = 0
  void ensureDeviceSegments(activeDeviceId.value)
})

watch(
  [() => currentPlayerUrl.value, () => playbackVisible.value, () => playbackMode.value],
  async ([url, visible]) => {
    if (!visible) {
      stopPlayer()
      return
    }
    if (!url) {
      stopPlayer()
      return
    }
    await nextTick()
    await attachPlayerSource()
  },
)

async function loadPlayback() {
  loading.value = true
  try {
    const response = await listPlayback({
      page: devicePage.value,
      page_size: devicePageSize.value,
    })
    devices.value = response.items
    totalDevices.value = response.total
    const nextActiveId = devices.value.find((item) => item.device_id === activeDeviceId.value)?.device_id
    activeDeviceId.value = nextActiveId ?? devices.value[0]?.device_id ?? ''
    await ensureDeviceSegments(activeDeviceId.value)
  } catch (error) {
    appToast.errorFrom('加载失败', error, '回放列表加载失败')
  } finally {
    loading.value = false
  }
}

async function ensureDeviceSegments(deviceId: string) {
  if (!deviceId) {
    return
  }
  segmentLoading.value = true
  try {
    const response = await listDevicePlaybackSegments(deviceId, {
      page: Math.floor(pageFirst.value / pageRows.value) + 1,
      page_size: pageRows.value,
    })
    deviceSegmentsMap.value = {
      ...deviceSegmentsMap.value,
      [deviceId]: response.items,
    }
    activeDeviceSegmentsTotal.value = response.total
  } catch (error) {
    appToast.errorFrom('片段加载失败', error, '设备片段加载失败')
  } finally {
    segmentLoading.value = false
  }
}

async function ensureTodayDeviceSegments(deviceId: string) {
  if (!deviceId || todayDeviceSegmentsMap.value[deviceId]) {
    return
  }
  try {
    const segments = await listTodayDevicePlaybackSegments(deviceId)
    todayDeviceSegmentsMap.value = {
      ...todayDeviceSegmentsMap.value,
      [deviceId]: segments,
    }
    const firstSegment = segments[0]
    if (activeDeviceId.value === deviceId) {
      timelineSecond.value = firstSegment ? segmentStartInDay(firstSegment) : 0
    }
  } catch (error) {
    appToast.errorFrom('时间轴加载失败', error, '当天时间轴片段加载失败')
  }
}

function formatStartTime(value: number) {
  return new Date(value * 1000).toLocaleString('zh-CN', { hour12: false })
}

function formatDateTime(value: string) {
  return new Date(value).toLocaleString('zh-CN', { hour12: false })
}

function formatDuration(value: number) {
  const totalSeconds = Math.max(0, Math.round(value))
  if (totalSeconds >= 60) {
    const minutes = Math.floor(totalSeconds / 60)
    const seconds = totalSeconds % 60
    return `${minutes}m ${seconds}s`
  }
  return `${totalSeconds}s`
}

function formatFileSize(value: number) {
  if (value >= 1024 * 1024 * 1024) {
    return `${(value / 1024 / 1024 / 1024).toFixed(2)} GB`
  }
  if (value >= 1024 * 1024) {
    return `${(value / 1024 / 1024).toFixed(2)} MB`
  }
  if (value >= 1024) {
    return `${(value / 1024).toFixed(2)} KB`
  }
  return `${value} B`
}

function formatVideoInfo(segment: PlaybackSegmentItem) {
  const size =
    segment.video_width > 0 && segment.video_height > 0
      ? `${segment.video_width}x${segment.video_height}`
      : '-'
  const fps = segment.video_fps > 0 ? `${segment.video_fps.toFixed(2)} fps` : '-'
  const codec = segment.video_codec || '-'
  const bitrate = segment.video_bit_rate > 0 ? `${segment.video_bit_rate} bps` : '-'
  return `${codec} / ${size} / ${fps} / ${bitrate}`
}

function formatAudioInfo(segment: PlaybackSegmentItem) {
  const codec = segment.audio_codec || '-'
  const sampleRate = segment.audio_sample_rate > 0 ? `${segment.audio_sample_rate} Hz` : '-'
  const channels = segment.audio_channels > 0 ? `${segment.audio_channels} ch` : '-'
  const bitrate = segment.audio_bit_rate > 0 ? `${segment.audio_bit_rate} bps` : '-'
  return `${codec} / ${sampleRate} / ${channels} / ${bitrate}`
}

function formatSegmentRange(segment: PlaybackSegmentItem) {
  return `${formatStartTime(segment.start_time)} - ${formatDateTime(
    new Date(segment.start_time * 1000 + segment.duration * 1000).toISOString(),
  )}`
}

function formatEndTime(segment: PlaybackSegmentItem) {
  return new Date(segment.start_time * 1000 + segment.duration * 1000).toLocaleString('zh-CN', {
    hour12: false,
  })
}

function segmentStartInDay(segment: PlaybackSegmentItem) {
  return Math.max(0, Math.floor((segment.start_time * 1000 - todayRange.value.startMs) / 1000))
}

function segmentEndInDay(segment: PlaybackSegmentItem) {
  return Math.min(DAY_SECONDS, segmentStartInDay(segment) + Math.ceil(segment.duration))
}

function buildTimelineBackground(segments: PlaybackSegmentItem[]) {
  if (!segments.length) {
    return 'linear-gradient(90deg, #d1d5db 0%, #d1d5db 100%)'
  }
  const stops: string[] = ['#d1d5db 0%']
  let current = 0
  for (const segment of segments) {
    const start = Math.max(current, segmentStartInDay(segment))
    const end = Math.max(start, segmentEndInDay(segment))
    const startPct = (start / DAY_SECONDS) * 100
    const endPct = (end / DAY_SECONDS) * 100
    if (start > current) {
      stops.push(`#d1d5db ${startPct}%`)
    }
    stops.push(`#2563eb ${startPct}%`)
    stops.push(`#38bdf8 ${endPct}%`)
    current = end
  }
  if (current < DAY_SECONDS) {
    stops.push(`#d1d5db ${(current / DAY_SECONDS) * 100}%`)
    stops.push('#d1d5db 100%')
  }
  return `linear-gradient(90deg, ${stops.join(', ')})`
}

async function ensureHlsJs() {
  if (window.Hls) {
    return window.Hls
  }

  await new Promise<void>((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>('script[data-hlsjs="true"]')
    if (existing) {
      existing.addEventListener('load', () => resolve(), { once: true })
      existing.addEventListener('error', () => reject(new Error('hls.js load failed')), { once: true })
      return
    }

    const script = document.createElement('script')
    script.src = 'https://cdn.jsdelivr.net/npm/hls.js@1.5.18/dist/hls.min.js'
    script.async = true
    script.dataset.hlsjs = 'true'
    script.onload = () => resolve()
    script.onerror = () => reject(new Error('hls.js load failed'))
    document.head.appendChild(script)
  })

  return window.Hls
}

function stopPlayer() {
  hlsPlayerRef.value?.destroy()
  hlsPlayerRef.value = null
  const player = playerRef.value
  if (!player) {
    return
  }
  player.pause()
  player.removeAttribute('src')
  player.load()
}

async function attachPlayerSource() {
  const player = playerRef.value
  if (!player || !currentPlayerUrl.value) {
    return
  }
  stopPlayer()

  if (playbackMode.value === 'playlist' || playbackMode.value === 'segment') {
    if (player.canPlayType('application/vnd.apple.mpegurl')) {
      player.src = currentPlayerUrl.value
      player.load()
      return
    }
    const Hls = await ensureHlsJs()
    if (!Hls?.isSupported()) {
      throw new Error('当前浏览器不支持 HLS 播放')
    }
    const hls = new Hls()
    hls.loadSource(currentPlayerUrl.value)
    hls.attachMedia(player)
    hlsPlayerRef.value = hls
    return
  }

}

function startTimelineDrag(event: PointerEvent) {
  isDraggingTimeline.value = true
  updateTimelineFromPointer(event.clientX)
  window.addEventListener('pointermove', onTimelineDrag)
  window.addEventListener('pointerup', stopTimelineDrag, { once: true })
}

function onTimelineDrag(event: PointerEvent) {
  updateTimelineFromPointer(event.clientX)
}

function stopTimelineDrag() {
  if (isDraggingTimeline.value) {
    isDraggingTimeline.value = false
    void openPlaylistPlayback(timelineSecond.value)
  }
  window.removeEventListener('pointermove', onTimelineDrag)
}

function updateTimelineFromPointer(clientX: number) {
  const track = timelineTrackRef.value
  if (!track) {
    return
  }
  const rect = track.getBoundingClientRect()
  const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
  timelineSecond.value = Math.round(ratio * DAY_SECONDS)
}

function resolveTimelineTarget(targetSecond: number) {
  const segment = findClosestSegment(targetSecond)
  if (!segment) {
    return null
  }
  const start = segmentStartInDay(segment)
  const end = segmentEndInDay(segment)
  const offset = targetSecond >= start && targetSecond <= end ? targetSecond - start : 0
  const daySecond = targetSecond >= start && targetSecond <= end ? targetSecond : start
  return {
    segment,
    offset,
    daySecond,
    playlistSecond: playlistSecondBySegment(segment.id, offset),
  }
}

function findClosestSegment(targetSecond: number) {
  if (!todaySegments.value.length) {
    return null
  }
  let best = todaySegments.value[0]
  let bestDistance = Number.POSITIVE_INFINITY
  for (const segment of todaySegments.value) {
    const start = segmentStartInDay(segment)
    const end = segmentEndInDay(segment)
    const distance =
      targetSecond < start ? start - targetSecond : targetSecond > end ? targetSecond - end : 0
    if (distance < bestDistance) {
      best = segment
      bestDistance = distance
      if (distance === 0) {
        break
      }
    }
  }
  return best
}

async function playSegment(segment: PlaybackSegmentItem, offsetSecond = 0, autoplay = true) {
  playbackMode.value = 'segment'
  const nextUrl = buildPlaybackSegmentPlaylistUrl(segment.id)
  pendingSeekSecond.value = Math.max(0, Math.min(offsetSecond, Math.max(segment.duration - 0.1, 0)))
  currentSegmentId.value = segment.id
  currentPlaybackSecond.value = pendingSeekSecond.value
  timelineSecond.value = segmentStartInDay(segment) + pendingSeekSecond.value
  playbackVisible.value = true

  if (currentPlayerUrl.value !== nextUrl) {
    currentPlayerUrl.value = nextUrl
    return
  }

  const player = playerRef.value
  if (!player) {
    return
  }
  player.currentTime = pendingSeekSecond.value
  if (autoplay) {
    try {
      await player.play()
    } catch {
      // ignore autoplay rejection
    }
  }
}

async function openPlaylistPlayback(targetSecond?: number) {
  if (!activeDevice.value) {
    return
  }
  await ensureTodayDeviceSegments(activeDevice.value.device_id)
  const firstSegment = todaySegments.value[0]
  const baseTarget =
    targetSecond ?? timelineSecond.value ?? (firstSegment ? segmentStartInDay(firstSegment) : 0)
  const resolved = resolveTimelineTarget(baseTarget)
  if (!resolved) {
    appToast.info('暂无片段', '当天没有可播放的录制片段', 1800)
    return
  }

  playbackMode.value = 'playlist'
  currentSegmentId.value = resolved.segment.id
  currentPlaybackSecond.value = resolved.offset
  timelineSecond.value = resolved.daySecond
  pendingSeekSecond.value = resolved.playlistSecond
  playbackVisible.value = true
  const nextUrl = buildPlaybackPlaylistUrl(activeDevice.value.device_id)
  if (currentPlayerUrl.value !== nextUrl) {
    currentPlayerUrl.value = nextUrl
    return
  }
  const player = playerRef.value
  if (!player) {
    return
  }
  player.currentTime = pendingSeekSecond.value
  try {
    await player.play()
  } catch {
    // ignore autoplay rejection
  }
}

function onVideoLoadedMetadata() {
  const player = playerRef.value
  if (!player) {
    return
  }
  if (pendingSeekSecond.value !== null) {
    player.currentTime = pendingSeekSecond.value
    pendingSeekSecond.value = null
  }
  if (currentSegmentId.value) {
    void player.play().catch(() => {})
  }
}

function onVideoTimeUpdate() {
  const player = playerRef.value
  if (!player || isDraggingTimeline.value) {
    return
  }
  if (playbackMode.value === 'playlist') {
    const state = timelineByPlaylistSecond(player.currentTime)
    if (!state) {
      return
    }
    currentSegmentId.value = state.segment.id
    currentPlaybackSecond.value = state.segmentSecond
    timelineSecond.value = state.daySecond
    return
  }
  if (!currentSegment.value) {
    return
  }
  currentPlaybackSecond.value = player.currentTime
  timelineSecond.value = Math.min(DAY_SECONDS, segmentStartInDay(currentSegment.value) + player.currentTime)
}

function onVideoEnded() {
  if (!currentSegment.value) {
    return
  }
  const currentIndex = todaySegments.value.findIndex((segment) => segment.id === currentSegment.value?.id)
  const nextSegment = currentIndex >= 0 ? todaySegments.value[currentIndex + 1] : null
  if (nextSegment) {
    void playSegment(nextSegment, 0, true)
  }
}

function currentPlayerTimeInSegment() {
  return currentPlaybackSecond.value
}

function resetPlayback(resetTimeline: boolean) {
  currentSegmentId.value = ''
  currentPlayerUrl.value = ''
  pendingSeekSecond.value = null
  if (resetTimeline) {
    timelineSecond.value = 0
  }
}

function openDetail(segment: PlaybackSegmentItem) {
  detailSegment.value = segment
  detailVisible.value = true
}

function onPage(event: { first: number; rows: number }) {
  pageFirst.value = event.first
  pageRows.value = event.rows
  deviceSegmentsMap.value = {
    ...deviceSegmentsMap.value,
    [activeDeviceId.value]: [],
  }
  void ensureDeviceSegments(activeDeviceId.value)
}

async function onDevicePage(event: { first: number; rows: number; page: number }) {
  devicePage.value = event.page + 1
  devicePageSize.value = event.rows
  await loadPlayback()
}

function closePlayback() {
  playbackVisible.value = false
  playerRef.value?.pause()
}

function playlistSecondBySegment(segmentId: string, offset = 0) {
  let total = 0
  for (const segment of todaySegments.value) {
    if (segment.id === segmentId) {
      return total + offset
    }
    total += segment.duration
  }
  return total
}

function timelineByPlaylistSecond(playlistSecond: number) {
  let total = 0
  for (const segment of todaySegments.value) {
    const nextTotal = total + segment.duration
    if (playlistSecond <= nextTotal) {
      const segmentSecond = Math.max(0, playlistSecond - total)
      return {
        segment,
        segmentSecond,
        daySecond: Math.min(DAY_SECONDS, segmentStartInDay(segment) + segmentSecond),
      }
    }
    total = nextTotal
  }
  const lastSegment = todaySegments.value[todaySegments.value.length - 1]
  if (!lastSegment) {
    return null
  }
  return {
    segment: lastSegment,
    segmentSecond: lastSegment.duration,
    daySecond: segmentEndInDay(lastSegment),
  }
}

function formatDaySecond(second: number) {
  const clamped = Math.max(0, Math.min(DAY_SECONDS, Math.floor(second)))
  const hours = String(Math.floor(clamped / 3600)).padStart(2, '0')
  const minutes = String(Math.floor((clamped % 3600) / 60)).padStart(2, '0')
  const seconds = String(clamped % 60).padStart(2, '0')
  return `${hours}:${minutes}:${seconds}`
}

</script>

<template>
  <div class="content-section playback-page">
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">回放</h1>
        <p class="page-subtitle">按设备查看录制片段，支持时间轴回放</p>
      </div>
      <div class="page-actions">
        <Tag severity="contrast" :value="`${totalDevices} 设备`" />
        <Tag severity="secondary" :value="`${totalSegments} 片段`" />
        <Button icon="pi pi-refresh" text aria-label="刷新" @click="loadPlayback" />
      </div>
    </div>

    <div v-if="!loading && !devices.length" class="empty-state">
      <i class="pi pi-video empty-state-icon" />
      <p class="empty-state-text">暂无设备或录制片段数据</p>
    </div>

    <Paginator
      v-if="totalDevices > devicePageSize"
      :first="(devicePage - 1) * devicePageSize"
      :rows="devicePageSize"
      :total-records="totalDevices"
      :rows-per-page-options="[12, 24, 48]"
      template="PrevPageLink PageLinks NextPageLink RowsPerPageDropdown"
      class="device-paginator"
      @page="onDevicePage"
    />

    <Tabs
      v-if="devices.length"
      v-model:value="activeDeviceId"
      scrollable
      class="playback-tabs"
    >
      <TabList>
        <Tab v-for="device in devices" :key="device.device_id" :value="device.device_id">
          <div class="device-tab">
            <span class="device-tab-title">{{ device.device_name }}</span>
            <Tag
              :severity="device.segment_count > 0 ? 'info' : 'secondary'"
              :value="String(device.segment_count)"
            />
          </div>
        </Tab>
      </TabList>
      <TabPanels>
        <TabPanel v-for="device in devices" :key="device.device_id" :value="device.device_id">
          <Card class="data-card">
            <template #header>
              <div class="card-header">
                <div class="device-info">
                  <div class="device-name">{{ device.device_name }}</div>
                  <div class="device-meta">
                    <span>{{ device.device_id }}</span>
                    <span>{{ device.input_type }}</span>
                  </div>
                </div>
                <Tag
                  :severity="activeDevice?.segment_count ? 'info' : 'secondary'"
                  :value="`${activeDevice?.segment_count ?? 0} 个片段`"
                />
              </div>
            </template>
            <template #content>
              <div class="segment-actions">
                <Button
                  icon="pi pi-window-maximize"
                  label="打开回放"
                  text
                  :disabled="!activeSegments.length"
                  @click="openPlaylistPlayback()"
                />
              </div>

              <div v-if="segmentLoading" class="segment-empty">
                <i class="pi pi-spin pi-spinner" />
                <span>录制片段加载中...</span>
              </div>

              <div v-else-if="!activeSegments.length" class="segment-empty">
                <i class="pi pi-video" />
                <span>当前设备暂无录制片段</span>
              </div>

              <div v-else class="segment-grid">
                <article
                  v-for="segment in pagedSegments"
                  :key="segment.id"
                  class="segment-card"
                  :class="{ 'segment-card-active': currentSegmentId === segment.id }"
                >
                  <button
                    type="button"
                    class="segment-preview"
                    @click="playSegment(segment, 0, true)"
                  >
                    <div class="segment-preview-placeholder">
                      <i class="pi pi-video segment-preview-placeholder-icon" />
                      <span class="segment-preview-placeholder-text">{{ formatStartTime(segment.start_time) }}</span>
                    </div>
                    <span class="segment-duration-badge">{{ formatDuration(segment.duration) }}</span>
                    <span class="segment-preview-overlay">
                      <i class="pi pi-play-circle" />
                      <span>播放片段</span>
                    </span>
                  </button>

                  <div class="segment-card-body">
                    <div class="segment-file-name" :title="segment.file_name">{{ segment.file_name }}</div>
                    <div class="segment-range">{{ formatSegmentRange(segment) }}</div>

                    <div class="segment-metrics">
                      <div class="segment-metric">
                        <span class="segment-metric-label">时长</span>
                        <span>{{ formatDuration(segment.duration) }}</span>
                      </div>
                      <div class="segment-metric">
                        <span class="segment-metric-label">大小</span>
                        <span>{{ formatFileSize(segment.file_size) }}</span>
                      </div>
                    </div>

                    <div class="segment-card-actions">
                      <Button
                        icon="pi pi-play"
                        label="播放"
                        size="small"
                        @click="playSegment(segment, 0, true)"
                      />
                      <Button
                        icon="pi pi-info-circle"
                        label="详情"
                        text
                        size="small"
                        @click="openDetail(segment)"
                      />
                    </div>
                  </div>
                </article>
              </div>

                <Paginator
                  v-if="activeDeviceSegmentsTotal > pageRows"
                  :first="pageFirst"
                  :rows="pageRows"
                  :total-records="activeDeviceSegmentsTotal"
                  :rows-per-page-options="[8, 12, 24]"
                  template="PrevPageLink PageLinks NextPageLink RowsPerPageDropdown"
                  class="segment-paginator"
                  @page="onPage"
                />
              </template>
            </Card>
          </TabPanel>
        </TabPanels>
      </Tabs>

    <Dialog
      v-model:visible="playbackVisible"
      modal
      maximizable
      dismissable-mask
      header="回放"
      class="playback-dialog"
      :style="{ width: '100vw', height: '100vh', maxWidth: '100vw', maxHeight: '100vh' }"
      :content-style="{ padding: '0', height: 'calc(100vh - 3.5rem)' }"
      @hide="closePlayback"
    >
      <div class="player-shell player-shell-dialog" :class="{ 'player-shell-segment': playbackMode === 'segment' }">
        <div class="player-panel player-panel-dialog">
          <video
            ref="playerRef"
            class="video-player"
            controls
            playsinline
            preload="metadata"
            @loadedmetadata="onVideoLoadedMetadata"
            @timeupdate="onVideoTimeUpdate"
            @ended="onVideoEnded"
          />
          <div v-if="!currentPlayerUrl" class="player-empty">
            点击下方播放按钮，或拖动当天时间轴开始回放。
          </div>
        </div>

        <div v-if="playbackMode === 'playlist'" class="timeline-card timeline-card-dialog">
          <div class="timeline-header">
            <div>
              <div class="timeline-title">当天时间轴</div>
              <div class="timeline-subtitle">
                蓝色表示有录制片段，灰色表示无数据。当前定位：{{ currentTimelineLabel }}。
                当前模式：{{ playbackMode === 'playlist' ? 'm3u8 连续回放' : 'TS 单片段播放' }}
              </div>
            </div>
            <div class="timeline-stats">
              <Tag severity="info" :value="`${todaySegments.length} 个片段`" />
              <Tag severity="contrast" :value="currentSegmentElapsed" />
            </div>
          </div>

          <div class="timeline-track-shell">
            <div class="timeline-scale">
              <span v-for="tick in TIMELINE_TICKS" :key="tick">
                {{ String(tick).padStart(2, '0') }}:00
              </span>
            </div>
            <div
              ref="timelineTrackRef"
              class="timeline-track"
              @pointerdown="startTimelineDrag"
            >
              <div class="timeline-track-background" :style="{ background: timelineBackground }" />
              <div
                class="timeline-marker"
                :style="{ left: `${(timelineSecond / DAY_SECONDS) * 100}%` }"
              >
                <span class="timeline-marker-line" />
                <span class="timeline-marker-dot" />
              </div>
            </div>
          </div>

          <div class="timeline-hint-row">
            <Tag severity="secondary" value="灰色: 无片段" />
            <Tag severity="info" value="蓝色: 有片段" />
            <Button
              icon="pi pi-history"
              label="使用当天连续回放"
              text
              size="small"
              :disabled="!todaySegments.length"
              @click="openPlaylistPlayback()"
            />
          </div>
        </div>

        <div v-else class="segment-player-footer">
          <Tag severity="info" value="单片段播放" />
          <span class="segment-player-footer-text">
            当前片段使用浏览器原生进度条和控制条。
          </span>
        </div>
      </div>
    </Dialog>

    <Dialog
      v-model:visible="detailVisible"
      modal
      header="片段详情"
      class="segment-detail-dialog"
      :style="{ width: 'min(42rem, calc(100vw - 2rem))' }"
    >
      <div v-if="detailSegment" class="detail-grid">
        <div class="detail-item">
          <span class="detail-label">文件名</span>
          <span class="mono-text">{{ detailSegment.file_name }}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">开始时间</span>
          <span>{{ formatStartTime(detailSegment.start_time) }}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">结束时间</span>
          <span>{{ formatEndTime(detailSegment) }}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">时长</span>
          <span>{{ formatDuration(detailSegment.duration) }}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">文件大小</span>
          <span>{{ formatFileSize(detailSegment.file_size) }}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">视频信息</span>
          <span>{{ formatVideoInfo(detailSegment) }}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">音频信息</span>
          <span>{{ formatAudioInfo(detailSegment) }}</span>
        </div>
        <div class="detail-item detail-item-full">
          <span class="detail-label">文件路径</span>
          <span class="mono-text">{{ detailSegment.file_path }}</span>
        </div>
        <div class="detail-item">
          <span class="detail-label">入库时间</span>
          <span>{{ formatDateTime(detailSegment.update_time) }}</span>
        </div>
      </div>
    </Dialog>
  </div>
</template>

<style scoped>
/* Page-specific styles - matching DashboardView style */

.data-card {
  animation: slide-up 0.5s ease-out 0.15s backwards;
}

.playback-tabs {
  animation: slide-up 0.5s ease-out 0.15s backwards;
}

:deep(.playback-tabs.p-tabs) {
  overflow: hidden;
  background:
    radial-gradient(circle at 8% 0%, rgb(59 130 246 / 12%), transparent 28rem),
    rgb(15 23 42 / 40%);
  backdrop-filter: blur(12px);
  border: 1px solid rgb(148 163 184 / 10%);
  border-radius: 0.875rem;
  box-shadow: 0 4px 12px rgb(0 0 0 / 20%);
}

:deep(.playback-tabs .p-tablist) {
  background: linear-gradient(180deg, rgb(30 41 59 / 52%), rgb(15 23 42 / 36%));
  border-color: rgb(148 163 184 / 10%);
}

:deep(.playback-tabs .p-tablist-content),
:deep(.playback-tabs .p-tablist-tab-list) {
  background: transparent;
}

:deep(.playback-tabs .p-tab) {
  color: #94a3b8;
  background: transparent;
  border-color: transparent;
  transition:
    color 0.16s ease,
    background 0.16s ease,
    border-color 0.16s ease;
}

:deep(.playback-tabs .p-tab:hover) {
  color: #dbeafe;
  background: rgb(59 130 246 / 10%);
}

:deep(.playback-tabs .p-tab.p-tab-active) {
  color: #60a5fa;
  background: rgb(59 130 246 / 14%);
  border-color: #3b82f6;
}

:deep(.playback-tabs .p-tablist-active-bar) {
  background: #3b82f6;
  box-shadow: 0 0 12px rgb(59 130 246 / 50%);
}

:deep(.playback-tabs .p-tablist-prev-button),
:deep(.playback-tabs .p-tablist-next-button) {
  color: #94a3b8;
  background: rgb(15 23 42 / 72%);
  border: 1px solid rgb(148 163 184 / 10%);
}

:deep(.playback-tabs .p-tablist-prev-button:hover),
:deep(.playback-tabs .p-tablist-next-button:hover) {
  color: #e2e8f0;
  background: rgb(148 163 184 / 12%);
}

:deep(.playback-tabs .p-tabpanels) {
  padding: 1rem;
  color: #e2e8f0;
  background: transparent;
}

:deep(.playback-tabs .p-tabpanel) {
  background: transparent;
}

.device-tab {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  max-width: 16rem;
}

.device-tab-title {
  max-width: 12rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 1rem;
  padding-bottom: 1rem;
  border-bottom: 1px solid rgb(148 163 184 / 10%);
}

.device-info {
  flex: 1;
}

.device-name {
  font-size: 0.9375rem;
  font-weight: 600;
  color: #e2e8f0;
  margin-bottom: 0.25rem;
}

.device-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  color: #64748b;
  font-size: 0.75rem;
}

.segment-actions {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 1rem;
}

.segment-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
  gap: 1rem;
}

.segment-card {
  overflow: hidden;
  border-radius: 1rem;
  border: 1px solid rgb(148 163 184 / 10%);
  background: rgb(15 23 42 / 40%);
  backdrop-filter: blur(12px);
  box-shadow: 0 4px 12px rgb(0 0 0 / 20%);
  transition: all 0.3s;
}

.segment-card:hover {
  border-color: rgb(148 163 184 / 20%);
  box-shadow: 0 8px 24px rgb(0 0 0 / 30%);
}

.segment-card-active {
  border-color: rgb(59 130 246 / 50%);
  box-shadow: 0 8px 24px rgb(59 130 246 / 30%);
}

.segment-preview {
  position: relative;
  display: block;
  width: 100%;
  padding: 0;
  border: 0;
  background: linear-gradient(160deg, rgb(15 23 42), rgb(30 41 59));
  cursor: pointer;
}

.segment-preview-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  aspect-ratio: 16 / 9;
  gap: 0.75rem;
  color: rgb(226 232 240);
  background:
    radial-gradient(circle at top left, rgb(59 130 246 / 28%), transparent 38%),
    linear-gradient(160deg, rgb(15 23 42), rgb(30 41 59));
}

.segment-preview-placeholder-icon {
  font-size: 2rem;
}

.segment-preview-placeholder-text {
  font-size: 0.875rem;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.segment-preview-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  color: #fff;
  background: linear-gradient(180deg, transparent, rgb(15 23 42 / 55%));
  opacity: 0;
  transition: opacity 0.18s ease;
}

.segment-preview:hover .segment-preview-overlay {
  opacity: 1;
}

.segment-duration-badge {
  position: absolute;
  top: 0.75rem;
  right: 0.75rem;
  padding: 0.25rem 0.5rem;
  border-radius: 999px;
  background: rgb(15 23 42 / 78%);
  color: #fff;
  font-size: 0.75rem;
  font-weight: 600;
}

.segment-card-body {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  padding: 1rem;
}

.segment-file-name {
  font-size: 0.875rem;
  font-weight: 600;
  color: #e2e8f0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.segment-range {
  font-size: 0.8125rem;
  color: #64748b;
  line-height: 1.5;
}

.segment-metrics {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.75rem;
}

.segment-metric {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
  font-size: 0.8125rem;
  color: #cbd5e1;
}

.segment-metric-label {
  color: #64748b;
  font-size: 0.6875rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.segment-card-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
}

/* Player and timeline styles */
.player-shell {
  display: grid;
  grid-template-columns: minmax(0, 1.25fr) minmax(20rem, 0.95fr);
  gap: 1rem;
  margin-bottom: 1rem;
}

.player-shell-dialog {
  height: 100%;
  margin-bottom: 0;
  padding: 1rem;
  grid-template-columns: 1fr;
  grid-template-rows: minmax(0, 1fr) auto;
}

.player-shell-segment {
  grid-template-rows: minmax(0, 1fr) auto;
}

:deep(.playback-dialog) {
  overflow: hidden;
  background:
    radial-gradient(circle at 10% 0%, rgb(59 130 246 / 16%), transparent 34rem),
    linear-gradient(145deg, rgb(2 6 23 / 99%), rgb(15 23 42 / 98%));
  border: 1px solid rgb(96 165 250 / 20%);
  box-shadow: 0 30px 90px rgb(2 6 23 / 72%);
}

:deep(.playback-dialog .p-dialog-header) {
  height: 3.5rem;
  padding: 0 1.25rem;
  background: linear-gradient(180deg, rgb(15 23 42 / 92%), rgb(15 23 42 / 76%));
  backdrop-filter: blur(12px);
  border-bottom: 1px solid rgb(148 163 184 / 12%);
  color: #e2e8f0;
}

:deep(.playback-dialog .p-dialog-title) {
  color: #e2e8f0;
  font-size: 0.95rem;
  font-weight: 700;
  letter-spacing: -0.01em;
}

:deep(.playback-dialog .p-dialog-header-actions) {
  gap: 0.25rem;
}

:deep(.playback-dialog .p-dialog-header-icon) {
  width: 2rem;
  height: 2rem;
  color: #94a3b8;
  border-radius: 0.55rem;
}

:deep(.playback-dialog .p-dialog-header-icon:hover) {
  color: #e2e8f0;
  background: rgb(148 163 184 / 12%);
}

:deep(.playback-dialog .p-dialog-content) {
  background:
    linear-gradient(180deg, rgb(15 23 42 / 42%), rgb(2 6 23 / 46%)),
    rgb(2 6 23);
  color: #cbd5e1;
}

.player-panel {
  position: relative;
  min-height: 20rem;
  border-radius: 1rem;
  overflow: hidden;
  background:
    radial-gradient(circle at top left, rgb(37 99 235 / 18%), transparent 42%),
    linear-gradient(160deg, rgb(15 23 42), rgb(17 24 39));
  border: 1px solid rgb(148 163 184 / 25%);
}

.player-panel-dialog {
  min-height: 0;
  height: 100%;
  border-color: rgb(96 165 250 / 20%);
  box-shadow:
    0 18px 54px rgb(2 6 23 / 58%),
    inset 0 1px 0 rgb(226 232 240 / 5%);
}

.video-player {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  height: 100%;
  background: transparent;
}

.player-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1.5rem;
  color: rgb(226 232 240);
  text-align: center;
}

.timeline-card {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  padding: 1rem;
  border-radius: 1rem;
  border: 1px solid rgb(148 163 184 / 10%);
  background: rgb(15 23 42 / 40%);
  backdrop-filter: blur(12px);
}

.timeline-card-dialog {
  min-height: 0;
  height: auto;
  background:
    radial-gradient(circle at 0% 0%, rgb(59 130 246 / 12%), transparent 22rem),
    rgb(15 23 42 / 64%);
  border-color: rgb(148 163 184 / 14%);
  box-shadow:
    0 14px 40px rgb(2 6 23 / 36%),
    inset 0 1px 0 rgb(226 232 240 / 5%);
}

.segment-player-footer {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0 0.25rem 0.25rem;
  color: #94a3b8;
  font-size: 0.8125rem;
}

.segment-player-footer-text {
  line-height: 1.5;
}

.timeline-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
}

.timeline-title {
  font-size: 0.9375rem;
  font-weight: 600;
  color: #e2e8f0;
}

.timeline-subtitle {
  margin-top: 0.25rem;
  color: #94a3b8;
  font-size: 0.8125rem;
  line-height: 1.5;
}

.timeline-stats {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.timeline-track-shell {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.timeline-scale {
  display: flex;
  justify-content: space-between;
  gap: 0.5rem;
  color: #64748b;
  font-size: 0.6875rem;
  font-weight: 500;
}

.timeline-track {
  position: relative;
  height: 2.75rem;
  cursor: pointer;
}

.timeline-track-background {
  position: absolute;
  left: 0;
  right: 0;
  top: 1.25rem;
  height: 0.5rem;
  border-radius: 999px;
  border: 1px solid rgb(148 163 184 / 20%);
  overflow: hidden;
}

.timeline-marker {
  position: absolute;
  top: 0;
  transform: translateX(-50%);
  pointer-events: none;
}

.timeline-marker-line {
  display: block;
  width: 2px;
  height: 2.1rem;
  margin: 0 auto;
  background: #3b82f6;
  box-shadow: 0 0 8px rgb(59 130 246 / 60%);
}

.timeline-marker-dot {
  display: block;
  width: 0.75rem;
  height: 0.75rem;
  margin: -0.15rem auto 0;
  border-radius: 999px;
  background: #3b82f6;
  box-shadow: 0 0 0 3px rgb(59 130 246 / 30%);
}

.timeline-hint-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.detail-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;
}

:deep(.segment-detail-dialog) {
  overflow: hidden;
  background:
    radial-gradient(circle at 12% 0%, rgb(59 130 246 / 14%), transparent 28rem),
    linear-gradient(145deg, rgb(15 23 42 / 98%), rgb(30 41 59 / 94%));
  border: 1px solid rgb(96 165 250 / 20%);
  border-radius: 1rem;
  box-shadow:
    0 28px 80px rgb(2 6 23 / 68%),
    inset 0 1px 0 rgb(226 232 240 / 8%);
}

:deep(.segment-detail-dialog .p-dialog-header) {
  padding: 1.25rem 1.35rem 1rem;
  background: linear-gradient(180deg, rgb(30 41 59 / 62%), rgb(15 23 42 / 0%));
  border-bottom: 1px solid rgb(148 163 184 / 12%);
}

:deep(.segment-detail-dialog .p-dialog-title) {
  color: #e2e8f0;
  font-size: 1rem;
  font-weight: 700;
}

:deep(.segment-detail-dialog .p-dialog-content) {
  padding: 1.35rem;
  background: transparent;
  color: #cbd5e1;
}

:deep(.segment-detail-dialog .p-dialog-header-close) {
  width: 2rem;
  height: 2rem;
  color: #94a3b8;
  border-radius: 0.55rem;
}

:deep(.segment-detail-dialog .p-dialog-header-close:hover) {
  color: #e2e8f0;
  background: rgb(148 163 184 / 12%);
}

.detail-item {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
  min-width: 0;
  padding: 0.875rem;
  color: #cbd5e1;
  background: rgb(15 23 42 / 42%);
  border: 1px solid rgb(148 163 184 / 10%);
  border-radius: 0.75rem;
}

.detail-item-full {
  grid-column: 1 / -1;
}

.detail-label {
  font-size: 0.75rem;
  color: var(--p-text-muted-color);
}

.segment-paginator {
  margin-top: 1rem;
}

@media (width <= 1024px) {
  .player-shell {
    grid-template-columns: 1fr;
  }

  .player-shell-dialog {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(0, 1fr) auto;
  }
}

@media (width <= 768px) {
  .page-header,
  .page-actions,
  .timeline-header,
  .timeline-stats,
  .segment-player-footer,
  .segment-card-actions,
  .detail-grid {
    display: flex;
    flex-direction: column;
  }

  .page-actions {
    align-items: stretch;
  }

  .segment-metrics {
    grid-template-columns: 1fr;
  }

  .detail-grid {
    gap: 0.75rem;
  }
}
</style>
