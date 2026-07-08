import { request } from './request'

// Server-side multi-view compositor: fuses several sources into ONE program
// stream published to ZLM. Endpoints return the shared { code, message, data }
// envelope, so the standard `request()` wrapper applies.

export interface CompositorRegion {
  /** The source id this region shows. */
  source: string
  x: number
  y: number
  w: number
  h: number
}

export interface CompositorSource {
  id: string
  url: string
}

export interface CompositorProgram {
  id: string
  sources: CompositorSource[]
  width: number
  height: number
  regions: CompositorRegion[]
  /** e.g. rtmp://127.0.0.1:8555/switcher/{id} — where the server publishes it. */
  publish_url: string
  fps: number
}

export interface CreateCompositorPayload {
  id: string
  sources: CompositorSource[]
  width?: number
  height?: number
  /** Explicit regions; omit for an automatic grid. */
  regions?: CompositorRegion[]
  fps?: number
  bitrate?: number
  publish_url?: string
}

export function createCompositor(payload: CreateCompositorPayload) {
  return request<CompositorProgram>('/compositor/create', { method: 'POST', body: payload })
}

export function listCompositors() {
  return request<CompositorProgram[]>('/compositor/list')
}

/** Live-switch region `region` of program `id` to source `to`. */
export function switchRegion(id: string, region: number, to: string) {
  return request<null>(`/compositor/switch/${encodeURIComponent(id)}`, {
    method: 'POST',
    body: { region, to },
  })
}

/**
 * Live-relayout program `id` to a new set of regions (canvas size unchanged).
 * The server rebuilds only its filter graph, keeping the encoder/muxer — so the
 * published stream keeps flowing and the browser preview never reconnects.
 */
export function relayoutCompositor(id: string, regions: CompositorRegion[]) {
  return request<null>(`/compositor/relayout/${encodeURIComponent(id)}`, {
    method: 'POST',
    body: { regions },
  })
}

export function removeCompositor(id: string) {
  return request<null>(`/compositor/remove/${encodeURIComponent(id)}`, { method: 'POST' })
}
