import type { OutputConfig } from '../api/device'
import type { FormErrors } from './detectionForm'

export const resolutionOptions = [
  { label: '保持输入分辨率', value: '' },
  ...[[3840, 2160], [2560, 1440], [1920, 1080], [1280, 720], [854, 480], [640, 360]].map(
    ([width, height]) => ({ label: `${width} × ${height}`, value: `${width}x${height}` }),
  ),
]
export const videoCodecOptions = [
  { label: '保持默认', value: '' }, { label: 'H.264', value: 'h264' }, { label: 'H.265', value: 'h265' },
]
export const audioCodecOptions = [
  { label: '保持默认', value: '' }, { label: 'AAC', value: 'aac' }, { label: 'Opus', value: 'opus' },
]
export function sampleRateOptions(codec: unknown): { label: string; value: number | null }[] {
  const rates = codec === 'opus' ? [8000, 12000, 16000, 24000, 48000]
    : [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000]
  return [{ label: '保持输入采样率', value: null }, ...rates.map(value => ({ label: `${value} Hz`, value }))]
}
export function outputSupported(inputType: unknown): boolean {
  return inputType !== 'gb28181' && inputType !== 'xiaomi'
}
export function outputFormValues(config?: OutputConfig): Record<string, unknown> {
  return {
    output_audio_bus_id: config?.audio_bus_id ?? '',
    output_audio_mode: config?.audio_mode ?? 'program',
    output_use_global_audio: config?.use_global_audio ?? true,
    output_video_codec: config?.video_codec ?? '',
    output_resolution: config?.width && config?.height ? `${config.width}x${config.height}` : '',
    output_fps: config?.fps ?? null,
    output_max_bitrate: config?.max_bitrate ? config.max_bitrate / 1000 : null,
    output_audio_codec: config?.audio_codec ?? '',
    output_sample_rate: config?.sample_rate ?? null,
  }
}
export function validateOutputValues(values: Record<string, unknown>): FormErrors {
  if (!outputSupported(values.input_type)) return {}
  const errors: FormErrors = {}
  for (const [field, min, max] of [['output_fps', 1, 120], ['output_max_bitrate', 32, 100000]] as const) {
    const value = values[field]
    if (value !== null && value !== undefined && value !== '' &&
      (!Number.isInteger(value) || Number(value) < min || Number(value) > max)) {
      errors[field] = [{ message: `请输入 ${min}～${max} 的整数` }]
    }
  }
  if (!['', 'h264', 'h265', undefined].some(value => value === values.output_video_codec)) {
    errors.output_video_codec = [{ message: '请选择 H.264 或 H.265' }]
  }
  if (!['', 'aac', 'opus', undefined].some(value => value === values.output_audio_codec)) {
    errors.output_audio_codec = [{ message: '请选择 AAC 或 Opus' }]
  }
  if (values.output_resolution && !resolutionOptions.some(option => option.value === values.output_resolution)) {
    errors.output_resolution = [{ message: '请选择输出分辨率' }]
  }
  if (values.output_sample_rate !== undefined && values.output_sample_rate !== null &&
    !sampleRateOptions(values.output_audio_codec).some(option => option.value === values.output_sample_rate)) {
    errors.output_sample_rate = [{ message: '请选择该编码支持的采样率' }]
  }
  return errors
}
export function buildOutputConfig(values: Record<string, unknown>): OutputConfig | undefined {
  const config: OutputConfig = {}
  if (typeof values.output_audio_bus_id === 'string' && values.output_audio_bus_id.trim()) {
    config.audio_bus_id = values.output_audio_bus_id
    config.audio_mode = values.output_audio_mode === 'add_original' ? 'add_original' : 'program'
  }
  if (values.output_use_global_audio === false) config.use_global_audio = false
  if (!outputSupported(values.input_type)) return Object.keys(config).length ? config : undefined
  if (values.output_video_codec === 'h264' || values.output_video_codec === 'h265') config.video_codec = values.output_video_codec
  if (values.output_audio_codec === 'aac' || values.output_audio_codec === 'opus') config.audio_codec = values.output_audio_codec
  const resolution = resolutionOptions.find(option => option.value === values.output_resolution)
  if (resolution?.value) {
    const [width, height] = resolution.value.split('x').map(Number)
    config.width = width
    config.height = height
  }
  if (values.output_fps) config.fps = Number(values.output_fps)
  if (values.output_max_bitrate) config.max_bitrate = Number(values.output_max_bitrate) * 1000
  if (values.output_sample_rate) config.sample_rate = Number(values.output_sample_rate)
  return Object.keys(config).length ? config : undefined
}
