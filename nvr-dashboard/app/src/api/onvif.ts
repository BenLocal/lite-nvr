import { request } from './request'

export interface OnvifDiscovered {
  endpoints: string[]
  name: string | null
  hardware: string | null
  addr: string | null
}
export interface OnvifProfile {
  token: string
  name: string
  width: number
  height: number
  video_codec: string
  fps: number
}
export interface OnvifDeviceInfo {
  manufacturer: string
  model: string
  firmware: string
  serial: string
}
export interface OnvifProbe {
  device_info: OnvifDeviceInfo
  profiles: OnvifProfile[]
}
export interface OnvifPreset {
  token: string
  name: string
}

export function discoverOnvif(timeoutMs = 3000) {
  return request<OnvifDiscovered[]>('/onvif/discover', {
    method: 'POST',
    body: { timeout_ms: timeoutMs },
  })
}

export function probeOnvif(payload: {
  host: string
  port: number
  username: string
  password: string
}) {
  return request<OnvifProbe>('/onvif/probe', { method: 'POST', body: payload })
}

export function onvifPtz(payload: {
  device_id: string
  direction: string
  speed?: number
  preset_token?: string
}) {
  return request<null>('/onvif/ptz', { method: 'POST', body: payload })
}

export function getOnvifPresets(deviceId: string) {
  return request<OnvifPreset[]>(`/onvif/presets/${encodeURIComponent(deviceId)}`)
}
