import { request } from './request'

export interface DetectConfig {
  enabled: boolean
  models: string[]
  sample_every_ms: number
  min_confidence: number
}

export interface OutputConfig {
  use_global_audio?: boolean
  audio_bus_id?: string
  audio_mode?: 'program' | 'add_original'
  video_codec?: 'h264' | 'h265'
  width?: number
  height?: number
  fps?: number
  max_bitrate?: number
  audio_codec?: 'aac' | 'opus'
  sample_rate?: number
}

export interface DeviceConfig {
  output?: OutputConfig
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
  state?: 'running' | 'starting' | 'idle'
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

export async function listDevices(): Promise<DeviceItem[]> {
  const devices = await request<DeviceItem[]>('/device/list')
  for (const device of devices) {
    if (device.config?.output !== undefined && !isOutputConfig(device.config.output)) {
      throw new Error('设备输出参数格式不正确')
    }
    if (device.state !== undefined && !['running', 'starting', 'idle'].includes(device.state)) {
      throw new Error('设备状态数据格式不正确')
    }
  }
  return devices
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
export interface TrafficMetrics {
  bytes_total: number
  packets_total: number
  bytes_per_second: number
  packets_per_second: number
  last_packet_age_ms: number | null
}
export interface MediaStreamMetrics {
  index: number
  kind: string
  codec: string
  width: number
  height: number
  fps: number | null
  pixel_format: string | null
  sample_rate: number
  channels: number
  traffic: TrafficMetrics
}
export interface CodecMetrics {
  stream_index: number
  kind: string
  codec: string
  hardware: boolean
  running: boolean
  /** Video decoder decoding keyframes only: no subscriber needs every frame. */
  key_frames_only?: boolean
  traffic: TrafficMetrics
}
export interface MediaOutputMetrics {
  id: string
  destination: string
  traffic_available: boolean
  stream: MediaStreamMetrics
}
export interface DevicePipelineMetrics {
  running: boolean
  input: MediaStreamMetrics[]
  decoders: CodecMetrics[]
  encoders: CodecMetrics[]
  output: MediaOutputMetrics[]
}
export interface DistributionTrack {
  codec_name: string
  is_video: boolean
  width: number
  height: number
  fps: number
  sample_rate: number
  channels: number
  bit_rate: number
}
export interface DeviceDistributionMetrics {
  schema: string
  app: string
  stream: string
  total_reader_count: number
  bytes_per_second: number
  tracks: DistributionTrack[]
}
export interface DeviceMetrics {
  id: string
  input_type: string
  state: 'running' | 'starting' | 'idle'
  message: string | null
  pipeline: DevicePipelineMetrics | null
  distribution: DeviceDistributionMetrics | null
  sampled_at_ms: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
function numbers(value: Record<string, unknown>, keys: string[]): boolean {
  return keys.every((key) => typeof value[key] === 'number' && Number.isFinite(value[key]))
}
function isTraffic(value: unknown): value is TrafficMetrics {
  return isRecord(value) && numbers(value, ['bytes_total', 'packets_total', 'bytes_per_second', 'packets_per_second']) &&
    (value.last_packet_age_ms === null || numbers(value, ['last_packet_age_ms']))
}
function isMediaStream(value: unknown): value is MediaStreamMetrics {
  return isRecord(value) && numbers(value, ['index', 'width', 'height', 'sample_rate', 'channels']) &&
    typeof value.kind === 'string' && typeof value.codec === 'string' &&
    (value.fps === null || numbers(value, ['fps'])) &&
    (value.pixel_format === null || typeof value.pixel_format === 'string') && isTraffic(value.traffic)
}
function isCodec(value: unknown): value is CodecMetrics {
  return isRecord(value) && numbers(value, ['stream_index']) && typeof value.kind === 'string' &&
    typeof value.codec === 'string' && typeof value.hardware === 'boolean' && typeof value.running === 'boolean' &&
    (value.key_frames_only === undefined || typeof value.key_frames_only === 'boolean') && isTraffic(value.traffic)
}
function isOutput(value: unknown): value is MediaOutputMetrics {
  return isRecord(value) && typeof value.id === 'string' && typeof value.destination === 'string' && typeof value.traffic_available === 'boolean' && isMediaStream(value.stream)
}
function isPipeline(value: unknown): value is DevicePipelineMetrics {
  return isRecord(value) && typeof value.running === 'boolean' && Array.isArray(value.input) && value.input.every(isMediaStream) &&
    Array.isArray(value.decoders) && value.decoders.every(isCodec) && Array.isArray(value.encoders) && value.encoders.every(isCodec) &&
    Array.isArray(value.output) && value.output.every(isOutput)
}
function isTrack(value: unknown): value is DistributionTrack {
  return isRecord(value) && typeof value.codec_name === 'string' && typeof value.is_video === 'boolean' &&
    numbers(value, ['width', 'height', 'fps', 'sample_rate', 'channels', 'bit_rate'])
}
function isDistribution(value: unknown): value is DeviceDistributionMetrics {
  return isRecord(value) && typeof value.schema === 'string' && typeof value.app === 'string' && typeof value.stream === 'string' &&
    numbers(value, ['total_reader_count', 'bytes_per_second']) && Array.isArray(value.tracks) && value.tracks.every(isTrack)
}
function isDeviceMetrics(value: unknown): value is DeviceMetrics {
  return isRecord(value) && typeof value.id === 'string' && typeof value.input_type === 'string' &&
    typeof value.state === 'string' && ['running', 'starting', 'idle'].includes(value.state) && (value.message === null || typeof value.message === 'string') &&
    (value.pipeline === null || isPipeline(value.pipeline)) && (value.distribution === null || isDistribution(value.distribution)) && numbers(value, ['sampled_at_ms'])
}
export function parseDeviceMetrics(value: unknown): DeviceMetrics[] {
  if (!Array.isArray(value) || !value.every(isDeviceMetrics)) throw new Error('设备流量数据格式不正确')
  return value
}
export async function getDeviceMetrics(signal?: AbortSignal): Promise<DeviceMetrics[]> {
  return parseDeviceMetrics(await request<unknown>('/device/metrics', { signal }))
}

function isOutputConfig(value: unknown): value is OutputConfig {
  if (!isRecord(value)) return false
  if (value.audio_bus_id !== undefined && (typeof value.audio_bus_id !== 'string' || !value.audio_bus_id.trim())) return false
  if (value.audio_mode !== undefined && value.audio_mode !== 'program' && value.audio_mode !== 'add_original') return false
  if (value.use_global_audio !== undefined && typeof value.use_global_audio !== 'boolean') return false
  if (value.video_codec !== undefined && value.video_codec !== 'h264' && value.video_codec !== 'h265') return false
  if (value.audio_codec !== undefined && value.audio_codec !== 'aac' && value.audio_codec !== 'opus') return false
  return ['width', 'height', 'fps', 'max_bitrate', 'sample_rate'].every(key =>
    value[key] === undefined || (typeof value[key] === 'number' && Number.isInteger(value[key]) && value[key] > 0),
  )
}
