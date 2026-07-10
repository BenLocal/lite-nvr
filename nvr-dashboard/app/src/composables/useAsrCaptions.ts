import { ref, shallowRef } from 'vue'
import type { Socket } from 'socket.io-client'
import { startAsr, stopAsr } from '../api/asr'

// socket.io-client is a bundled dependency loaded via a dynamic import, so Vite
// code-splits it into its own chunk served from our own origin — lazy-loaded on
// first use and available offline (the NVR is typically deployed on an isolated
// network), instead of a public CDN.
async function ensureIo() {
  return (await import('socket.io-client')).io
}

export interface CaptionEntry {
  id: number
  time: string // wall-clock HH:MM:SS
  text: string
}

function stamp(): string {
  const d = new Date()
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

/**
 * Live captions for one pipe: connects to the `/asr` Socket.IO namespace, joins
 * the pipe's room, drives `POST /api/asr/{pipe}/start|stop`, and exposes the
 * finalized lines plus the in-flight partial as reactive state.
 */
export function useAsrCaptions() {
  const active = ref(false)
  const loading = ref(false)
  const error = ref('')
  const entries = ref<CaptionEntry[]>([])
  const partial = ref('')
  const socket = shallowRef<Socket | null>(null)
  let seq = 0
  let currentPipe = ''

  async function start(pipeId: string) {
    if (active.value || loading.value) {
      return
    }
    error.value = ''
    loading.value = true
    currentPipe = pipeId
    try {
      const io = await ensureIo()
      const s = io('/asr')
      socket.value = s
      s.on('partial', (payload) => {
        const p = payload as { pipe?: string; text?: string }
        if (p?.pipe === currentPipe) {
          partial.value = p.text ?? ''
        }
      })
      s.on('final', (payload) => {
        const p = payload as { pipe?: string; text?: string }
        if (p?.pipe === currentPipe && p.text) {
          entries.value.push({ id: ++seq, time: stamp(), text: p.text })
          partial.value = ''
        }
      })
      // Join the room first so no early transcript is missed, then start.
      s.emit('subscribe', pipeId)
      await startAsr(pipeId)
      active.value = true
    } catch (e) {
      error.value = e instanceof Error ? e.message : '开启字幕失败'
      teardown()
    } finally {
      loading.value = false
    }
  }

  async function stop() {
    if (!active.value && !socket.value) {
      return
    }
    const pipeId = currentPipe
    active.value = false
    partial.value = ''
    try {
      if (pipeId) {
        await stopAsr(pipeId)
      }
    } catch {
      // Best-effort: the socket teardown below still frees the client.
    }
    teardown()
  }

  /** Drop the socket without touching the backend (used on error/unmount). */
  function teardown() {
    const s = socket.value
    if (s) {
      try {
        if (currentPipe) {
          s.emit('unsubscribe', currentPipe)
        }
      } catch {
        // ignore
      }
      try {
        s.disconnect()
      } catch {
        // ignore
      }
    }
    socket.value = null
  }

  function reset() {
    entries.value = []
    partial.value = ''
    error.value = ''
  }

  return { active, loading, error, entries, partial, start, stop, teardown, reset }
}
