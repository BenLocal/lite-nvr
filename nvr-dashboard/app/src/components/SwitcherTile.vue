<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { createStreamPlayer, type StreamPlayerHandle } from '../utils/streamPlayer'
import type { DeviceItem } from '../api/device'

const props = defineProps<{
  source: DeviceItem | null
  index: number
  active: boolean
}>()

const emit = defineEmits<{
  (e: 'select', index: number): void
  (e: 'enlarge', index: number): void
  (e: 'clear', index: number): void
}>()

const videoRef = ref<HTMLVideoElement | null>(null)
const jessibucaRef = ref<HTMLDivElement | null>(null)
const handle = ref<StreamPlayerHandle | null>(null)
const error = ref('')
// Bumped on every stop()/start() so an in-flight async start that is superseded
// by a newer one tears itself down instead of attaching a stale player.
let gen = 0

// Same URL rule as DeviceListView: prefer the backend-provided flv_url (GB28181
// streams live under /media/rtp), fall back to the /media/live proxy path.
function flvUrl(source: DeviceItem): string {
  return source.flv_url || `/media/live/${encodeURIComponent(source.id)}.live.flv`
}

watch(
  () => props.source?.id,
  async () => {
    stop()
    if (!props.source) {
      return
    }
    await nextTick()
    await start()
  },
  { immediate: true },
)

onBeforeUnmount(stop)

async function start() {
  stop()
  error.value = ''
  const video = videoRef.value
  const container = jessibucaRef.value
  const source = props.source
  if (!video || !container || !source) {
    return
  }

  const myGen = ++gen
  const url = flvUrl(source)
  try {
    const created = await createStreamPlayer(
      { video, container },
      url,
      { muted: true, onError: (m) => (error.value = m) },
    )
    if (myGen !== gen) {
      // Superseded by a newer start()/stop() while we were awaiting.
      created.destroy()
      return
    }
    handle.value = created
  } catch (e) {
    error.value = e instanceof Error ? e.message : '播放失败'
  }
}

function stop() {
  gen++
  if (handle.value) {
    handle.value.destroy()
    handle.value = null
  }
}
</script>

<template>
  <div
    class="tile"
    :class="{ 'is-active': active, 'is-empty': !source }"
    @click="emit('select', index)"
    @dblclick="source && emit('enlarge', index)"
  >
    <div v-show="source" class="tile-media">
      <video
        ref="videoRef"
        class="tile-video"
        muted
        autoplay
        playsinline
      />
      <div ref="jessibucaRef" class="tile-jessibuca" />
    </div>

    <div v-if="!source" class="tile-placeholder">
      <i class="pi pi-plus-circle" />
      <span>{{ active ? '点击左侧信号源分配' : '空画面' }}</span>
    </div>

    <span class="tile-index">{{ index + 1 }}</span>

    <div v-if="source" class="tile-bar">
      <span class="tile-live"><span class="tile-dot" />LIVE</span>
      <span class="tile-name ellipsis-text">{{ source.name || source.id }}</span>
      <button class="tile-clear" title="清空此画面" @click.stop="emit('clear', index)">
        <i class="pi pi-times" />
      </button>
    </div>

    <div v-if="error" class="tile-error">{{ error }}</div>
  </div>
</template>

<style scoped>
.tile {
  position: relative;
  overflow: hidden;
  min-height: 0;
  border: 1px solid rgb(148 163 184 / 14%);
  border-radius: 0.6rem;
  background:
    radial-gradient(circle at top, rgb(41 98 255 / 10%), transparent 60%),
    linear-gradient(180deg, rgb(10 17 29), rgb(17 23 33));
  cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s;
}

.tile:hover {
  border-color: rgb(59 130 246 / 45%);
}

.tile.is-active {
  border-color: #3b82f6;
  box-shadow: 0 0 0 2px rgb(59 130 246 / 55%), 0 6px 18px rgb(0 0 0 / 35%);
}

.tile.is-empty {
  border-style: dashed;
}

.tile-media {
  position: absolute;
  inset: 0;
}

.tile-video {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: contain;
  background: #0b111d;
}

/* Jessibuca mounts its own canvas here; shown only when it is the active backend. */
.tile-jessibuca {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  display: none;
  background: #0b111d;
}

.tile-placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.4rem;
  color: #64748b;
  font-size: 0.78rem;
}

.tile-placeholder i {
  font-size: 1.4rem;
}

.tile-index {
  position: absolute;
  top: 0.35rem;
  left: 0.4rem;
  z-index: 2;
  min-width: 1.1rem;
  height: 1.1rem;
  padding: 0 0.3rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 0.3rem;
  background: rgb(15 23 42 / 70%);
  color: #cbd5e1;
  font-size: 0.7rem;
  font-weight: 600;
}

.tile-bar {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: 2;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.45rem;
  background: linear-gradient(0deg, rgb(2 6 15 / 82%), transparent);
}

.tile-live {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  flex: none;
  font-size: 0.62rem;
  font-weight: 700;
  letter-spacing: 0.05em;
  color: #f87171;
}

.tile-dot {
  width: 0.4rem;
  height: 0.4rem;
  border-radius: 50%;
  background: #ef4444;
  box-shadow: 0 0 6px rgb(239 68 68 / 70%);
}

.tile-name {
  flex: 1;
  min-width: 0;
  font-size: 0.75rem;
  color: #e2e8f0;
}

.tile-clear {
  flex: none;
  width: 1.3rem;
  height: 1.3rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 0.3rem;
  background: rgb(148 163 184 / 12%);
  color: #cbd5e1;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s, background 0.15s;
}

.tile-clear:hover {
  background: rgb(239 68 68 / 25%);
  color: #fecaca;
}

.tile:hover .tile-clear {
  opacity: 1;
}

.tile-error {
  position: absolute;
  inset: auto 0.4rem 2rem;
  z-index: 3;
  padding: 0.3rem 0.45rem;
  border-radius: 0.3rem;
  background: rgb(127 29 29 / 55%);
  color: #fecaca;
  font-size: 0.72rem;
  text-align: center;
}
</style>
