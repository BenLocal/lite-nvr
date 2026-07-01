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
