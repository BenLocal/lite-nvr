// Loader for Jessibuca v3, a WASM soft-decode player used as the fallback when
// the browser can't hardware-decode a codec (e.g. HEVC) via MSE/mpegts.js.
//
// Jessibuca is not an npm module — its v3 build is a plain global script plus a
// worker decoder + wasm. We vendor those three files under `public/jessibuca/`
// so they are served from our own origin (at `${BASE_URL}jessibuca/…`), never a
// CDN. The script is injected once on first use and cached.

export interface JessibucaConfig {
  container: HTMLElement
  /** URL of the worker decoder (decoder.js), which loads decoder.wasm beside it. */
  decoder?: string
  videoBuffer?: number
  isResize?: boolean
  /** false → decode in WASM (software); the reason to use Jessibuca at all. */
  useMSE?: boolean
  useWCS?: boolean
  hasAudio?: boolean
  /** Jessibuca is muted unless this is true. */
  isNotMute?: boolean
  autoWasm?: boolean
  loadingText?: string
  operateBtns?: Record<string, boolean>
}

export interface JessibucaPlayer {
  play: (url: string) => void
  destroy: () => void
  pause?: () => void
  mute?: () => void
  clearView?: () => void
  on: (event: string, handler: (payload?: unknown) => void) => void
}

export type JessibucaCtor = new (config: JessibucaConfig) => JessibucaPlayer

const BASE = import.meta.env.BASE_URL
export const JESSIBUCA_SCRIPT = `${BASE}jessibuca/jessibuca.js`
export const JESSIBUCA_DECODER = `${BASE}jessibuca/decoder.js`

function globalCtor(): JessibucaCtor | undefined {
  return (window as unknown as { Jessibuca?: JessibucaCtor }).Jessibuca
}

let loadPromise: Promise<JessibucaCtor | undefined> | null = null

/** Load the vendored Jessibuca script once and resolve to its constructor. */
export function ensureJessibuca(): Promise<JessibucaCtor | undefined> {
  const existing = globalCtor()
  if (existing) {
    return Promise.resolve(existing)
  }
  if (!loadPromise) {
    loadPromise = new Promise<JessibucaCtor | undefined>((resolve, reject) => {
      const tag = document.querySelector<HTMLScriptElement>('script[data-jessibuca="true"]')
      if (tag) {
        tag.addEventListener('load', () => resolve(globalCtor()), { once: true })
        tag.addEventListener('error', () => reject(new Error('jessibuca 加载失败')), { once: true })
        return
      }
      const script = document.createElement('script')
      script.src = JESSIBUCA_SCRIPT
      script.async = true
      script.dataset.jessibuca = 'true'
      script.onload = () => resolve(globalCtor())
      script.onerror = () => reject(new Error('jessibuca 加载失败'))
      document.head.appendChild(script)
    })
  }
  return loadPromise
}
