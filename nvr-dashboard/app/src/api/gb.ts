import { request } from './request'

export interface GbDevice {
  device_id: string
  online: boolean
}

export interface GbChannel {
  channel_id: string
  name: string
  status: string
}

export function getGbDevices() {
  return request<GbDevice[]>('/gb/devices')
}

export function getGbCatalog(deviceId: string) {
  return request<GbChannel[]>(`/gb/catalog/${encodeURIComponent(deviceId)}`)
}
