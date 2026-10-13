import { describe, expect, it } from 'vitest'
import { buildDevicePayload, deviceFormInitialValues } from './deviceForm'
import { buildOutputConfig, outputFormValues, sampleRateOptions, validateOutputValues } from './outputForm'

const settings = { video_codec: 'h265', width: 640, height: 360, fps: 15, max_bitrate: 500000, audio_codec: 'opus', sample_rate: 24000 } as const

describe('persisted output settings', () => {
  it('selects program audio independently for each device, including native sources', () => {
    for (const input_type of ['rtsp', 'v4l2', 'gb28181', 'xiaomi']) {
      const values = { input_type, output_audio_bus_id: 'main', include_audio: true }
      const config = buildOutputConfig(values)
      expect(config).toEqual({ audio_bus_id: 'main', audio_mode: 'add_original' })
      expect(outputFormValues(config)).toMatchObject({ output_audio_bus_id: 'main' })
    }
    expect(buildOutputConfig({ input_type: 'rtsp', output_audio_bus_id: 'main', include_audio: false })).toEqual({ audio_bus_id: 'main', audio_mode: 'program' })
    expect(buildOutputConfig({ input_type: 'rtsp', output_audio_bus_id: '' })).toBeUndefined()
  })
  it('keeps per-output program opt-out even for native forwarding devices', () => {
    expect(buildOutputConfig({ input_type: 'gb28181', output_use_global_audio: false })).toEqual({ use_global_audio: false })
    expect(buildOutputConfig({ input_type: 'rtsp', output_use_global_audio: true })).toBeUndefined()
  })
  it('round trips all settings and converts Kbps to bps', () => {
    const values: Record<string, unknown> = { ...deviceFormInitialValues(null), input_type: 'rtsp', ...outputFormValues(settings) }
    expect(values.output_max_bitrate).toBe(500)
    expect(buildOutputConfig(values)).toEqual(settings)
    expect(buildDevicePayload(values, { gbDeviceId: '', gbChannelId: '', supportedDetectionInputTypes: [] }).config?.output).toEqual(settings)
  })
  it('keeps unset parameters optional and rejects invalid values', () => {
    expect(buildOutputConfig({ input_type: 'rtsp' })).toBeUndefined()
    expect(validateOutputValues({ input_type: 'rtsp', output_fps: 0, output_max_bitrate: -1, output_audio_codec: 'opus', output_sample_rate: 44100 })).toHaveProperty('output_sample_rate')
    expect(validateOutputValues({ input_type: 'rtsp', output_fps: 15, output_max_bitrate: 500 })).toEqual({})
  })
  it('offers codec-specific sample rates and excludes native forwarding', () => {
    expect(sampleRateOptions('aac').some(option => option.value === 44100)).toBe(true)
    expect(sampleRateOptions('opus').some(option => option.value === 44100)).toBe(false)
    expect(buildOutputConfig({ input_type: 'gb28181', ...outputFormValues(settings) })).toBeUndefined()
  })
})
