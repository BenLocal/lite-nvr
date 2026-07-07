import { request } from './request'

/** Which browser player the dashboard uses for live playback. */
export type PlayerBackend = 'mpegts' | 'jessibuca' | 'auto'

export interface DashboardSettings {
  player: PlayerBackend
}

export function getSettings() {
  return request<DashboardSettings>('/system/settings')
}

export function saveSettings(settings: DashboardSettings) {
  return request<DashboardSettings>('/system/settings', { method: 'POST', body: settings })
}
