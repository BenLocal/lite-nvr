<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import Button from 'primevue/button'
import Message from 'primevue/message'
import { useAsrCaptions } from '../composables/useAsrCaptions'

const props = defineProps<{ pipeId: string }>()

const { active, loading, error, entries, partial, start, stop, teardown, reset } = useAsrCaptions()
const listRef = ref<HTMLElement | null>(null)

async function toggle() {
  if (active.value) {
    await stop()
  } else {
    reset()
    await start(props.pipeId)
  }
}

// Keep the newest line in view.
watch(
  () => [entries.value.length, partial.value] as const,
  async () => {
    await nextTick()
    const el = listRef.value
    if (el) {
      el.scrollTop = el.scrollHeight
    }
  },
)

// Switching devices while the dialog stays open: stop the old pipe, clear log.
watch(
  () => props.pipeId,
  async () => {
    if (active.value) {
      await stop()
    }
    reset()
  },
)

onBeforeUnmount(() => {
  // Fire-and-forget: stop the backend tap if running, else just drop the socket.
  if (active.value) {
    void stop()
  } else {
    teardown()
  }
})
</script>

<template>
  <div class="transcript-panel">
    <div class="transcript-head">
      <span class="transcript-title">
        <span class="transcript-dot" :class="{ 'is-live': active }" />
        实时转写
      </span>
      <Button
        :label="active ? '关闭字幕' : '开启字幕'"
        icon="pi pi-comment"
        size="small"
        :outlined="!active"
        :severity="active ? 'primary' : 'secondary'"
        :loading="loading"
        @click="toggle"
      />
    </div>

    <Message v-if="error" severity="error" :closable="false" class="transcript-error">
      {{ error }}
    </Message>

    <div ref="listRef" class="transcript-list">
      <template v-if="entries.length || partial">
        <div v-for="e in entries" :key="e.id" class="transcript-line">
          <span class="transcript-time">{{ e.time }}</span>
          <span class="transcript-text">{{ e.text }}</span>
        </div>
        <div v-if="partial" class="transcript-line transcript-partial">
          <span class="transcript-time">…</span>
          <span class="transcript-text">{{ partial }}</span>
        </div>
      </template>
      <div v-else class="transcript-empty">
        {{ active ? '正在聆听…' : '点击「开启字幕」开始实时转写' }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.transcript-panel {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
  margin-top: 0.75rem;
  padding: 0.75rem;
  border: 1px solid rgb(148 163 184 / 12%);
  border-radius: 0.75rem;
  background: rgb(15 23 42 / 40%);
}

.transcript-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.transcript-title {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.9rem;
  font-weight: 600;
  color: #e2e8f0;
}

.transcript-dot {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: #64748b;
}

.transcript-dot.is-live {
  background: #3b82f6;
  box-shadow: 0 0 0 0 rgb(59 130 246 / 60%);
  animation: transcript-pulse 1.6s ease-out infinite;
}

@keyframes transcript-pulse {
  70% {
    box-shadow: 0 0 0 0.35rem rgb(59 130 246 / 0%);
  }

  100% {
    box-shadow: 0 0 0 0 rgb(59 130 246 / 0%);
  }
}

.transcript-error {
  margin: 0;
}

.transcript-list {
  height: 12rem;
  overflow-y: auto;
  padding: 0.5rem;
  border-radius: 0.5rem;
  background: rgb(30 41 59 / 40%);
  font-size: 0.85rem;
  line-height: 1.6;
}

.transcript-line {
  display: flex;
  gap: 0.6rem;
  padding: 0.15rem 0;
}

.transcript-time {
  flex: none;
  font-family: SFMono-Regular, Consolas, 'Liberation Mono', monospace;
  font-size: 0.75rem;
  color: #64748b;
  user-select: none;
}

.transcript-text {
  color: #e2e8f0;
  overflow-wrap: break-word;
}

.transcript-partial .transcript-text {
  color: #94a3b8;
  font-style: italic;
}

.transcript-empty {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #64748b;
  font-size: 0.85rem;
}
</style>
