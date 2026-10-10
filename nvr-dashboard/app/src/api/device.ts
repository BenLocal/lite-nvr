import { request } from './request'

export interface DetectConfig {
  enabled: boolean
  models: string[]
  sample_every_ms: number
  min_confidence: number
}

export interface DeviceConfig {
  detect?: DetectConfig
}

export interface DeviceItem {
  id: string
  name: string
  input_type: string
  input_value: string
  description: string
  include_audio: boolean
  record: boolean
  config?: DeviceConfig
  created_at: string
  updated_at: string
  flv_url?: string
}

export interface DevicePayload {
  id?: string
  name: string
  input_type: string
  input_value: string
  description?: string
  include_audio?: boolean
  record?: boolean
  config?: DeviceConfig
}

export function listDevices() {
  return request<DeviceItem[]>('/device/list')
}

// The backend has no single-device endpoint; resolve it from the list.
export async function getDevice(id: string): Promise<DeviceItem | undefined> {
  const devices = await listDevices()
  return devices.find((device) => device.id === id)
}

export function addDevice(payload: DevicePayload) {
  return request<DeviceItem>('/device/add', {
    method: 'POST',
    body: payload,
  })
}

export function updateDevice(id: string, payload: DevicePayload) {
  return request<DeviceItem>(`/device/update/${encodeURIComponent(id)}`, {
    method: 'POST',
    body: payload,
  })
}

export function removeDevice(id: string) {
  return request<string>(`/device/remove/${encodeURIComponent(id)}`, {
    method: 'POST',
  })
}
