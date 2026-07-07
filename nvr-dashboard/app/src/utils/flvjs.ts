// Loader for flv.js, bundled from the npm package (no CDN). It is pulled in via
// a dynamic import on first use, so it lands in its own chunk (kept out of the
// initial bundle) and is loaded at most once — the resolved factory is cached
// and shared across every player.

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

let cached: FlvJs | undefined

/** Load flv.js once (from the npm bundle) and resolve to its factory. */
export async function ensureFlvJs(): Promise<FlvJs | undefined> {
  if (!cached) {
    const mod = await import('flv.js')
    // flv.js exposes its factory as the module's default export.
    cached = (mod.default ?? mod) as unknown as FlvJs
  }
  return cached
}
