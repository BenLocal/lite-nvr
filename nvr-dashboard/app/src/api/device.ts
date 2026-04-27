import { request } from './request'

export interface DeviceItem {
  id: string
  name: string
  input_type: string
  input_value: string
  description: string
  include_audio: boolean
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
}

export function listDevices() {
  return request<DeviceItem[]>('/device/list')
}

export function addDevice(payload: DevicePayload) {
  return request<DeviceItem>('/device/add', {
    method: 'POST',
    body: payload,
  })
}

export function updateDevice(id: string, payload: DevicePayload) {
  return request<DeviceItem>(`/device/update/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: payload,
  })
}

export function removeDevice(id: string) {
  return request<string>(`/device/remove/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  })
}
