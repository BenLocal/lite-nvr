import { ensureMpegts, type MpegtsPlayer } from './mpegts'
import { ensureJessibuca, JESSIBUCA_DECODER, type JessibucaPlayer } from './jessibuca'
import { resolvePlayerBackend } from '../composables/usePlayerPreference'

export interface StreamPlayerHost {
  /** <video> element mpegts.js (MSE) renders into. */
  video: HTMLVideoElement
  /** Element Jessibuca mounts its canvas into. */
  container: HTMLElement
}

export interface StreamPlayerOptions {
  /** Mute audio (default true — these are live previews). */
  muted?: boolean
  onMediaInfo?: (info: Record<string, unknown>) => void
  onStats?: (info: Record<string, unknown>) => void
  onError?: (message: string) => void
}

export interface StreamPlayerHandle {
  /** Backend actually playing right now. */
  readonly backend: 'mpegts' | 'jessibuca'
  destroy: () => void
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : {}
}

/**
 * Start a live player bound to `host`, choosing the backend from the saved
 * dashboard preference:
 *   - `mpegts`    → mpegts.js (MSE, hardware decode)
 *   - `jessibuca` → Jessibuca (WASM, software decode)
 *   - `auto`      → mpegts.js first, fall back to Jessibuca on playback error
 * Toggles which host element is visible and returns a handle to tear it down.
 */
export async function createStreamPlayer(
  host: StreamPlayerHost,
  url: string,
  opts: StreamPlayerOptions = {},
): Promise<StreamPlayerHandle> {
  const backendPref = await resolvePlayerBackend()
  const muted = opts.muted ?? true

  let disposed = false
  let activeKind: 'mpegts' | 'jessibuca' = 'mpegts'
  let teardown: (() => void) | null = null

  const showVideo = () => {
    host.video.style.display = ''
    host.container.style.display = 'none'
  }
  const showContainer = () => {
    host.video.style.display = 'none'
    host.container.style.display = ''
  }

  function dropActive() {
    try {
      teardown?.()
    } catch {
      // ignore teardown errors
    }
    teardown = null
  }

  async function startMpegts(allowFallback: boolean): Promise<void> {
    const mpegts = await ensureMpegts()
    if (disposed) {
      return
    }
    activeKind = 'mpegts'
    showVideo()
    if (!mpegts?.isSupported()) {
      // Last resort: let the browser try the URL directly.
      host.video.src = url
      void host.video.play().catch(() => {})
      teardown = () => {
        host.video.pause()
        host.video.removeAttribute('src')
        host.video.load()
      }
      return
    }
    const p: MpegtsPlayer = mpegts.createPlayer({ type: 'flv', url, isLive: true })
    p.on?.('media_info', (info) => opts.onMediaInfo?.(asRecord(info)))
    p.on?.('statistics_info', (info) => opts.onStats?.(asRecord(info)))
    p.on?.('error', (payload) => {
      if (disposed) {
        return
      }
      if (allowFallback) {
        // auto mode: the MSE/hardware path failed → switch to WASM soft decode.
        dropActive()
        void startJessibuca()
      } else {
        opts.onError?.(String(payload ?? 'mpegts error'))
      }
    })
    p.attachMediaElement(host.video)
    p.load()
    void p.play().catch(() => {})
    teardown = () => {
      p.pause?.()
      p.unload?.()
      p.detachMediaElement?.()
      p.destroy()
      host.video.removeAttribute('src')
    }
  }

  async function startJessibuca(): Promise<void> {
    let ctor
    try {
      ctor = await ensureJessibuca()
    } catch (e) {
      opts.onError?.(e instanceof Error ? e.message : 'jessibuca 加载失败')
      return
    }
    if (disposed) {
      return
    }
    if (!ctor) {
      opts.onError?.('jessibuca 不可用')
      return
    }
    activeKind = 'jessibuca'
    showContainer()
    const jbc: JessibucaPlayer = new ctor({
      container: host.container,
      decoder: JESSIBUCA_DECODER,
      videoBuffer: 0.2,
      isResize: true,
      useMSE: false,
      useWCS: false,
      hasAudio: !muted,
      isNotMute: !muted,
      autoWasm: true,
      operateBtns: {
        fullscreen: false,
        screenshot: false,
        play: false,
        audio: false,
        record: false,
      },
    })
    jbc.on('videoInfo', (info) => opts.onMediaInfo?.(asRecord(info)))
    jbc.on('kBps', (rate) => opts.onStats?.({ speed: rate }))
    jbc.on('error', (payload) => opts.onError?.(String(payload ?? 'jessibuca error')))
    jbc.play(url)
    teardown = () => jbc.destroy()
  }

  if (backendPref === 'jessibuca') {
    await startJessibuca()
  } else if (backendPref === 'auto') {
    await startMpegts(true)
  } else {
    await startMpegts(false)
  }

  return {
    get backend() {
      return activeKind
    },
    destroy() {
      disposed = true
      dropActive()
      showVideo()
    },
  }
}
