// Runtime loader for flv.js. The library is pulled from a CDN on demand (kept
// out of the bundle) and cached on `window.flvjs`; a single <script> tag is
// shared across every player, so N tiles never load the script N times.

export type FlvPlayer = {
  attachMediaElement: (element: HTMLVideoElement) => void
  load: () => void
  play: () => Promise<void>
  on?: (event: string, listener: (payload: unknown) => void) => void
  pause?: () => void
  unload?: () => void
  detachMediaElement?: () => void
  destroy: () => void
}

export type FlvJs = {
  isSupported: () => boolean
  createPlayer: (mediaDataSource: { type: 'flv'; url: string; isLive: boolean }) => FlvPlayer
}

const FLVJS_CDN = 'https://cdn.jsdelivr.net/npm/flv.js@1.6.2/dist/flv.min.js'

function globalFlv(): FlvJs | undefined {
  return (window as unknown as { flvjs?: FlvJs }).flvjs
}

/** Load flv.js once and resolve to the global `flvjs` factory. */
export async function ensureFlvJs(): Promise<FlvJs | undefined> {
  const existing = globalFlv()
  if (existing) {
    return existing
  }

  await new Promise<void>((resolve, reject) => {
    const tag = document.querySelector<HTMLScriptElement>('script[data-flvjs="true"]')
    if (tag) {
      tag.addEventListener('load', () => resolve(), { once: true })
      tag.addEventListener('error', () => reject(new Error('flv.js load failed')), { once: true })
      return
    }

    const script = document.createElement('script')
    script.src = FLVJS_CDN
    script.async = true
    script.dataset.flvjs = 'true'
    script.onload = () => resolve()
    script.onerror = () => reject(new Error('flv.js load failed'))
    document.head.appendChild(script)
  })

  return globalFlv()
}
