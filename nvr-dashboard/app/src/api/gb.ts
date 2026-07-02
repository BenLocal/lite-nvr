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

export interface PtzRequest {
  device_id: string
  channel_id: string
  command: string
  speed?: number
  preset?: number
}

export function ptzControl(payload: PtzRequest) {
  return request<null>('/gb/ptz', {
    method: 'POST',
    body: payload,
  })
}

export interface GbStreamRtp {
  exist: boolean
  peer_ip: string
  peer_port: number
  local_port: number
  identifier: string
}

export interface GbStream {
  stream_id: string
  device_id: string
  channel_id: string
  transport: string
  live: boolean
  rtp: GbStreamRtp | null
}

export function getGbStreams() {
  return request<GbStream[]>('/gb/streams')
}
