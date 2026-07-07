import { ref, shallowRef } from 'vue'
import { startAsr, stopAsr } from '../api/asr'

// Minimal shape of the socket.io-client we use. Loaded at runtime from a CDN,
// so it isn't an npm dependency.
interface AsrSocket {
  emit: (event: string, ...args: unknown[]) => void
  on: (event: string, cb: (payload: unknown) => void) => void
  disconnect: () => void
}
type IoFactory = (uri: string, opts?: Record<string, unknown>) => AsrSocket

declare global {
  interface Window {
    io?: IoFactory
  }
}

const SOCKETIO_CDN = 'https://cdn.jsdelivr.net/npm/socket.io-client@4.8.1/dist/socket.io.min.js'

/** Load socket.io-client from CDN once and return the global `io` factory. */
async function ensureIo(): Promise<IoFactory> {
  if (window.io) {
    return window.io
  }
  await new Promise<void>((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>('script[data-socketio="true"]')
    if (existing) {
      existing.addEventListener('load', () => resolve(), { once: true })
      existing.addEventListener('error', () => reject(new Error('socket.io-client 加载失败')), {
        once: true,
      })
      return
    }
    const script = document.createElement('script')
    script.src = SOCKETIO_CDN
    script.async = true
    script.dataset.socketio = 'true'
    script.onload = () => resolve()
    script.onerror = () => reject(new Error('socket.io-client 加载失败'))
    document.head.appendChild(script)
  })
  if (!window.io) {
    throw new Error('socket.io-client 加载后未找到全局 io')
  }
  return window.io
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
  const socket = shallowRef<AsrSocket | null>(null)
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
