import { request } from './request'

export interface OverviewDevice {
  id: string
  name: string
  input_type: string
  description: string
  online: boolean
  record: boolean
  flv_url: string
}

export interface SystemOverview {
  device_total: number
  device_online: number
  device_offline: number
  record_segment_count: number
  record_total_bytes: number
  devices: OverviewDevice[]
}

export function getOverview() {
  return request<SystemOverview>('/system/overview')
}

/** Live host performance metrics, sampled server-side into a cache. All byte
 * fields are bytes; `net_*_bps` are bytes/sec over the sampler's window. */
export interface SystemMetrics {
  cpu_usage: number
  cpu_core_count: number
  mem_used: number
  mem_total: number
  swap_used: number
  swap_total: number
  net_rx_bps: number
  net_tx_bps: number
  net_rx_total: number
  net_tx_total: number
  load_one: number
  load_five: number
  load_fifteen: number
  sampled_at_ms: number
}

export function getMetrics() {
  return request<SystemMetrics>('/system/metrics')
}

/** A `/dev/video*` node on the nvr host. `capture`: nvr can read it
 * (single-planar video capture); false for metadata nodes and for
 * multi-planar-only nodes, which FFmpeg's v4l2 input cannot read. */
export interface V4l2Node {
  path: string
  name: string
  driver: string
  bus: string
  capture: boolean
  /** Only the multi-planar capture API (e.g. Rockchip rkcif / hdmirx). */
  mplane: boolean
  /** Why the node could not be queried (e.g. no permission). */
  error?: string
}

export function listV4l2Nodes() {
  return request<V4l2Node[]>('/system/list/v4l2/devices')
}

/** An X11 display on the nvr host that x11grab can capture. */
export interface X11Display {
  display: string
  /** It is the nvr server's own DISPLAY. */
  current: boolean
}

export function listX11Displays() {
  return request<X11Display[]>('/system/list/x11grab/devices')
}
