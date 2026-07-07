// Loader for mpegts.js (the maintained successor to flv.js; plays FLV and
// MPEG-TS), bundled from the npm package — no CDN. It is pulled in via a dynamic
// import on first use, so it lands in its own chunk (kept out of the initial
// bundle) and is loaded at most once — the resolved factory is cached and
// shared across every player.

export type MpegtsPlayer = {
  attachMediaElement: (element: HTMLVideoElement) => void
  load: () => void
  play: () => Promise<void>
  on?: (event: string, listener: (payload: unknown) => void) => void
  pause?: () => void
  unload?: () => void
  detachMediaElement?: () => void
  destroy: () => void
}

export type Mpegts = {
  isSupported: () => boolean
  createPlayer: (mediaDataSource: { type: 'flv'; url: string; isLive: boolean }) => MpegtsPlayer
}

let cached: Mpegts | undefined

/** Load mpegts.js once (from the npm bundle) and resolve to its factory. */
export async function ensureMpegts(): Promise<Mpegts | undefined> {
  if (!cached) {
    const mod = await import('mpegts.js')
    // mpegts.js exposes its factory as the module's default export.
    cached = (mod.default ?? mod) as unknown as Mpegts
  }
  return cached
}
