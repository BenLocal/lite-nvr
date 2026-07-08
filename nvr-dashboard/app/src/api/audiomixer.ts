import { request } from './request'

// Server-side audio mixing console: mixes several device audio streams into one
// or more independently-published output buses. Endpoints return the shared
// { code, message, data } envelope, so the standard `request()` wrapper applies.

export interface MixerInput {
  /** Device id feeding this bus. */
  source_id: string
  /** Volume percent (100 = unity). */
  volume: number
  muted: boolean
}

export interface MixerBus {
  id: string
  /** Where the server publishes the mixed stream (credentials redacted). */
  publish_url: string
  /** FLV path for playing the mixed output on the dashboard. */
  flv_url: string
  inputs: MixerInput[]
}

export interface MixerSource {
  id: string
  url: string
}

export interface MixerState {
  sources: MixerSource[]
  buses: MixerBus[]
}

export interface CreateBusInput {
  source_id: string
  volume?: number
}

export interface CreateBusPayload {
  id: string
  publish_url?: string
  inputs: CreateBusInput[]
}

export function getMixer() {
  return request<MixerState>('/audiomixer/list')
}

export function createBus(payload: CreateBusPayload) {
  return request<MixerState>('/audiomixer/bus/create', { method: 'POST', body: payload })
}

export function removeBus(bus_id: string) {
  return request<null>('/audiomixer/bus/remove', { method: 'POST', body: { bus_id } })
}

export function addBusInput(bus_id: string, source_id: string, volume?: number) {
  return request<null>('/audiomixer/bus/input/add', {
    method: 'POST',
    body: { bus_id, source_id, volume },
  })
}

export function removeBusInput(bus_id: string, source_id: string) {
  return request<null>('/audiomixer/bus/input/remove', {
    method: 'POST',
    body: { bus_id, source_id },
  })
}

export function setBusInputVolume(bus_id: string, source_id: string, volume: number) {
  return request<null>('/audiomixer/bus/input/volume', {
    method: 'POST',
    body: { bus_id, source_id, volume },
  })
}

export function setBusInputMute(bus_id: string, source_id: string, muted: boolean) {
  return request<null>('/audiomixer/bus/input/mute', {
    method: 'POST',
    body: { bus_id, source_id, muted },
  })
}
