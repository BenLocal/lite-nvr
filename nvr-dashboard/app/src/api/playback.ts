import { request } from './request'

export interface PlaybackSegmentItem {
  id: string
  start_time: number
  duration: number
  file_size: number
  file_name: string
  file_path: string
  video_codec: string
  video_width: number
  video_height: number
  video_fps: number
  video_bit_rate: number
  audio_codec: string
  audio_sample_rate: number
  audio_channels: number
  audio_bit_rate: number
  create_time: string
  update_time: string
}

export interface DevicePlaybackItem {
  device_id: string
  device_name: string
  input_type: string
  segment_count: number
}

export interface PlaybackListResponse {
  items: DevicePlaybackItem[]
  page: number
  page_size: number
  total: number
}

export function listPlayback(params: { page?: number; page_size?: number } = {}) {
  const search = new URLSearchParams()
  if (params.page) {
    search.set('page', String(params.page))
  }
  if (params.page_size) {
    search.set('page_size', String(params.page_size))
  }
  const suffix = search.size ? `?${search.toString()}` : ''
  return request<PlaybackListResponse>(`/playback/device/list${suffix}`)
}

export interface PlaybackSegmentsResponse {
  items: PlaybackSegmentItem[]
  page: number
  page_size: number
  total: number
}

export function listDevicePlaybackSegments(
  deviceId: string,
  params: { page?: number; page_size?: number } = {},
) {
  const search = new URLSearchParams()
  if (params.page) {
    search.set('page', String(params.page))
  }
  if (params.page_size) {
    search.set('page_size', String(params.page_size))
  }
  const suffix = search.size ? `?${search.toString()}` : ''
  return request<PlaybackSegmentsResponse>(`/playback/device/${encodeURIComponent(deviceId)}/segments${suffix}`)
}

export function listTodayDevicePlaybackSegments(deviceId: string) {
  return request<PlaybackSegmentItem[]>(`/playback/device/${encodeURIComponent(deviceId)}/today`)
}

export function buildPlaybackSegmentUrl(id: string) {
  return `/api/playback/segment/${encodeURIComponent(id)}`
}

export function buildPlaybackSegmentPlaylistUrl(id: string) {
  return `/api/playback/segment-playlist/${encodeURIComponent(id)}`
}

export function buildPlaybackPlaylistUrl(deviceId: string) {
  return `/api/playback/playlist/${encodeURIComponent(deviceId)}`
}
