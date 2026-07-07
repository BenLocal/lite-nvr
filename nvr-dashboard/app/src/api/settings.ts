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

/** Record-segment retention cleanup policy (applied by a periodic worker). */
export interface CleanupConfig {
  enabled: boolean
  /** Delete segments older than this many days (0 = off). */
  max_age_days: number
  /** Keep total recording size under this many GiB, prune oldest first (0 = off). */
  max_total_gb: number
  /** How often the worker runs, in minutes. */
  interval_minutes: number
}

export function getCleanup() {
  return request<CleanupConfig>('/system/cleanup')
}

export function saveCleanup(config: CleanupConfig) {
  return request<CleanupConfig>('/system/cleanup', { method: 'POST', body: config })
}
