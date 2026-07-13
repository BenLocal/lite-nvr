import { getAuthToken } from '../auth/token'
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

export interface DeleteSegmentsResult {
  deleted: number
}

export function deletePlaybackSegment(id: string) {
  return request<DeleteSegmentsResult>(`/playback/segment/${encodeURIComponent(id)}/delete`, {
    method: 'POST',
  })
}

export function deletePlaybackSegments(ids: string[]) {
  return request<DeleteSegmentsResult>('/playback/segments/delete', {
    method: 'POST',
    body: { ids },
  })
}

export function deleteAllDeviceSegments(deviceId: string) {
  return request<DeleteSegmentsResult>(
    `/playback/device/${encodeURIComponent(deviceId)}/segments/delete`,
    { method: 'POST' },
  )
}

// Player URLs carry the session token as a query param: hls.js segment
// fetches and Safari-native HLS can't reliably attach the Authorization
// header, and the backend accepts either channel (and echoes the token into
// the m3u8 segment URIs it generates).
function withAuthToken(url: string) {
  const token = getAuthToken()
  return token ? `${url}?token=${encodeURIComponent(token)}` : url
}

export function buildPlaybackSegmentUrl(id: string) {
  return withAuthToken(`/api/playback/segment/${encodeURIComponent(id)}`)
}

export function buildPlaybackSegmentPlaylistUrl(id: string) {
  return withAuthToken(`/api/playback/segment-playlist/${encodeURIComponent(id)}`)
}

export function buildPlaybackPlaylistUrl(deviceId: string) {
  return withAuthToken(`/api/playback/playlist/${encodeURIComponent(deviceId)}`)
}
