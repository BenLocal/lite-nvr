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
  channel_map?: ChannelMap
}

export type ChannelMap = 'stereo' | 'swap' | 'left_to_both' | 'right_to_both'

export interface AudioProfile {
  codec: 'aac' | 'opus'
  sample_rate: number
  channels: 2
  bitrate: number
}

export interface ProgramConfig {
  enabled: boolean
  mode: 'program' | 'add_original'
  active_bus_id: string | null
  profile: AudioProfile
  version: number
}

export interface ProgramState {
  config: ProgramConfig
  outputs: { device_id: string; use_global_audio: boolean; effective_version: number | null; state: 'original' | 'waiting' | 'applied' }[]
  last_error?: string | null
}

export interface AudioSourceOption { id: string; name: string; kind: 'device' | 'alsa' }

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function profile(value: unknown): value is AudioProfile {
  return record(value) && (value.codec === 'aac' || value.codec === 'opus') && value.channels === 2 &&
    typeof value.sample_rate === 'number' && (value.codec === 'opus' ? [8000, 12000, 16000, 24000, 48000] : [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000]).includes(value.sample_rate) &&
    typeof value.bitrate === 'number' && Number.isInteger(value.bitrate) && value.bitrate >= 16000 && value.bitrate <= 512000
}

function config(value: unknown): value is ProgramConfig {
  return record(value) && typeof value.enabled === 'boolean' && (value.mode === 'program' || value.mode === 'add_original') &&
    (value.active_bus_id === null || typeof value.active_bus_id === 'string') && profile(value.profile) &&
    typeof value.version === 'number' && Number.isSafeInteger(value.version) && value.version >= 0
}

function programState(value: unknown): value is ProgramState {
  return record(value) && config(value.config) && (value.last_error === undefined || value.last_error === null || typeof value.last_error === 'string') && Array.isArray(value.outputs) && value.outputs.every(output =>
    record(output) && typeof output.device_id === 'string' && typeof output.use_global_audio === 'boolean' &&
    (output.effective_version === null || (typeof output.effective_version === 'number' && Number.isSafeInteger(output.effective_version))) &&
    (output.state === 'original' || output.state === 'waiting' || output.state === 'applied'),
  )
}

export function parseProgramState(value: unknown): ProgramState {
  if (!programState(value)) throw new Error('混音台节目配置格式不正确')
  return value
}

export async function getProgramConfig(signal?: AbortSignal): Promise<ProgramState> {
  return parseProgramState(await request<unknown>('/audiomixer/config', { signal }))
}

export async function saveProgramConfig(config: ProgramConfig): Promise<ProgramState> {
  return parseProgramState(await request<unknown>('/audiomixer/config', { method: 'POST', body: config }))
}

function sourceOption(value: unknown): value is AudioSourceOption {
  return record(value) && typeof value.id === 'string' && typeof value.name === 'string' && (value.kind === 'device' || value.kind === 'alsa')
}

export async function getAudioSources(): Promise<AudioSourceOption[]> {
  const sources = await request<unknown>('/audiomixer/sources')
  if (!Array.isArray(sources) || !sources.every(sourceOption)) throw new Error('音频源列表格式不正确')
  return sources
}

export function setBusInputChannels(bus_id: string, source_id: string, channel_map: ChannelMap): Promise<null> {
  return request<null>('/audiomixer/bus/input/channels', { method: 'POST', body: { bus_id, source_id, channel_map } })
}

export interface MixerBus {
  enabled: boolean
  profile: AudioProfile
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
  ready?: boolean
  error?: string | null
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
  profile?: AudioProfile
  id: string
  publish_url?: string
  inputs: CreateBusInput[]
}

function mixerInput(value: unknown): value is MixerInput {
  return record(value) && typeof value.source_id === 'string' && typeof value.volume === 'number' &&
    Number.isInteger(value.volume) && value.volume >= 0 && value.volume <= 400 && typeof value.muted === 'boolean' &&
    (value.channel_map === undefined || ['stereo', 'swap', 'left_to_both', 'right_to_both'].some(mapping => mapping === value.channel_map))
}

function mixerSource(value: unknown): value is MixerSource {
  return record(value) && typeof value.id === 'string' && typeof value.url === 'string' &&
    (value.ready === undefined || typeof value.ready === 'boolean') &&
    (value.error === undefined || value.error === null || typeof value.error === 'string')
}

export function parseMixerState(value: unknown): MixerState {
  if (!record(value) || !Array.isArray(value.sources) || !value.sources.every(mixerSource) || !Array.isArray(value.buses)) {
    throw new Error('混音台状态格式不正确')
  }
  const buses: MixerBus[] = value.buses.map(bus => {
    if (!record(bus) || typeof bus.id !== 'string' || typeof bus.publish_url !== 'string' || typeof bus.flv_url !== 'string' ||
      !Array.isArray(bus.inputs) || !bus.inputs.every(mixerInput) || (bus.enabled !== undefined && typeof bus.enabled !== 'boolean') || (bus.profile !== undefined && !profile(bus.profile))) {
      throw new Error('混音总线格式不正确')
    }
    // Older servers have no per-bus profile and publish with the default AAC profile.
    const audioProfile = profile(bus.profile) ? bus.profile : { codec: 'aac', sample_rate: 48000, channels: 2, bitrate: 128000 } satisfies AudioProfile
    return { enabled: bus.enabled !== false, id: bus.id, publish_url: bus.publish_url, flv_url: bus.flv_url, inputs: bus.inputs, profile: audioProfile }
  })
  return { sources: value.sources, buses }
}

export async function getMixer(signal?: AbortSignal): Promise<MixerState> {
  return parseMixerState(await request<unknown>('/audiomixer/list', { signal }))
}

export async function createBus(payload: CreateBusPayload): Promise<MixerState> {
  return parseMixerState(await request<unknown>('/audiomixer/bus/create', { method: 'POST', body: payload }))
}

export async function startBus(payload: CreateBusPayload): Promise<MixerState> {
  return parseMixerState(await request<unknown>('/audiomixer/bus/start', { method: 'POST', body: payload }))
}

export async function stopBus(bus_id: string): Promise<MixerState> {
  return parseMixerState(await request<unknown>('/audiomixer/bus/stop', { method: 'POST', body: { bus_id } }))
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
