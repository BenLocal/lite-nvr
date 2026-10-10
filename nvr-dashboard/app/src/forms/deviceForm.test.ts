import { describe, expect, test } from 'vitest'
import type { DeviceItem } from '../api/device'
import {
  buildDevicePayload,
  deviceFormInitialValues,
  inputSettingsKind,
  inputTypeOptions,
  resolveDeviceForm,
  urlInputMeta,
} from './deviceForm'

const DEVICE: DeviceItem = {
  id: 'cam-1',
  name: 'Camera',
  input_type: 'rtsp',
  input_value: 'rtsp://camera/live',
  description: '',
  include_audio: false,
  record: true,
  config: {
    detect: {
      enabled: true,
      models: ['yolo'],
      sample_every_ms: 500,
      min_confidence: 0.4,
    },
  },
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

describe('device form helpers', () => {
  test('hydrates persisted detection values without sharing the model array', () => {
    const values = deviceFormInitialValues(DEVICE)
    expect(values).toMatchObject({
      name: 'Camera',
      detect_enabled: true,
      detect_models: ['yolo'],
      detect_sample_every_ms: 500,
      detect_min_confidence: 0.4,
    })
    ;(values.detect_models as string[]).push('changed')
    expect(DEVICE.config?.detect?.models).toEqual(['yolo'])
  })

  test('resolves GB selection into the submitted input value', () => {
    const result = resolveDeviceForm(
      {
        values: {
          ...deviceFormInitialValues(null),
          name: 'GB Camera',
          input_type: 'gb28181',
        },
      },
      {
        gbDeviceId: 'gb-device',
        gbChannelId: 'channel-1',
        validateDetection: false,
        maxDetectionSampleIntervalMs: 3_600_000,
      },
    )
    expect(result.errors).toEqual({})
    expect(JSON.parse(String(result.values.input_value))).toEqual({
      device_id: 'gb-device',
      channel_id: 'channel-1',
    })
  })

  test('merges base and detection validation errors', () => {
    const result = resolveDeviceForm(
      {
        values: {
          ...deviceFormInitialValues(null),
          name: ' ',
          input_value: '',
          detect_sample_every_ms: 3_600_001,
        },
      },
      {
        gbDeviceId: '',
        gbChannelId: '',
        validateDetection: true,
        maxDetectionSampleIntervalMs: 3_600_000,
      },
    )
    expect(result.errors).toHaveProperty('name')
    expect(result.errors).toHaveProperty('input_value')
    expect(result.errors).toHaveProperty('detect_sample_every_ms')
  })

  test('builds an immutable payload with detection config', () => {
    const models = ['yolo']
    const payload = buildDevicePayload(
      {
        ...deviceFormInitialValues(null),
        name: 'Camera',
        input_type: 'rtsp',
        input_value: 'rtsp://camera/live',
        detect_enabled: true,
        detect_models: models,
      },
      {
        gbDeviceId: '',
        gbChannelId: '',
        supportedDetectionInputTypes: ['rtsp'],
      },
    )
    models.push('changed')
    expect(payload.config?.detect?.models).toEqual(['yolo'])
    expect(payload.input_value).toBe('rtsp://camera/live')
  })

  test('hydrates structured Xiaomi and ONVIF inputs and tolerates malformed JSON', () => {
    const xiaomi = deviceFormInitialValues({
      ...DEVICE,
      input_type: 'xiaomi',
      input_value: JSON.stringify({
        user_id: 'user',
        token: 'token',
        region: 'de',
        did: 'did',
        model: 'model',
        ip: '10.0.0.1',
      }),
    })
    expect(xiaomi).toMatchObject({ xm_user_id: 'user', xm_region: 'de', xm_ip: '10.0.0.1' })

    const onvif = deviceFormInitialValues({
      ...DEVICE,
      input_type: 'onvif',
      input_value: JSON.stringify({ host: 'camera', port: 8899, profile_token: 'main' }),
    })
    expect(onvif).toMatchObject({
      onvif_host: 'camera',
      onvif_port: 8899,
      onvif_profile_token: 'main',
    })
    expect(deviceFormInitialValues({ ...DEVICE, input_type: 'xiaomi', input_value: '{' })).toMatchObject({
      xm_user_id: '',
    })
    expect(deviceFormInitialValues({ ...DEVICE, input_type: 'onvif', input_value: '{' })).toMatchObject({
      onvif_host: '',
    })
  })

  test('validates structured input variants at their form boundary', () => {
    const defaults = deviceFormInitialValues(null)
    const options = {
      gbDeviceId: '',
      gbChannelId: '',
      validateDetection: false,
      maxDetectionSampleIntervalMs: 3_600_000,
    }
    const xiaomi = resolveDeviceForm(
      { values: { ...defaults, name: 'Xiaomi', input_type: 'xiaomi' } },
      options,
    )
    expect(xiaomi.errors).toMatchObject({
      xm_user_id: expect.any(Array),
      xm_token: expect.any(Array),
      xm_did: expect.any(Array),
      xm_model: expect.any(Array),
      xm_ip: expect.any(Array),
    })

    const onvif = resolveDeviceForm(
      { values: { ...defaults, name: 'ONVIF', input_type: 'onvif' } },
      options,
    )
    expect(onvif.errors).toHaveProperty('onvif_host')
    expect(onvif.errors).toHaveProperty('input_value')

    const gbMissingChannel = resolveDeviceForm(
      { values: { ...defaults, name: 'GB', input_type: 'gb28181' } },
      { ...options, gbDeviceId: 'platform' },
    )
    expect(gbMissingChannel.errors.input_value?.[0]?.message).toBe('请选择国标通道')
  })

  test('serializes each structured payload without attaching detection to unsupported inputs', () => {
    const defaults = deviceFormInitialValues(null)
    const options = {
      gbDeviceId: 'platform',
      gbChannelId: 'channel',
      supportedDetectionInputTypes: ['rtsp'],
    }
    const xiaomi = buildDevicePayload(
      {
        ...defaults,
        name: 'Xiaomi',
        input_type: 'xiaomi',
        xm_user_id: 'user',
        xm_token: 'token',
        xm_did: 'did',
        xm_model: 'model',
        xm_ip: '10.0.0.1',
      },
      options,
    )
    expect(JSON.parse(xiaomi.input_value)).toMatchObject({ user_id: 'user', did: 'did' })
    expect(xiaomi.config).toBeUndefined()

    const gb = buildDevicePayload(
      { ...defaults, name: 'GB', input_type: 'gb28181' },
      options,
    )
    expect(JSON.parse(gb.input_value)).toEqual({ device_id: 'platform', channel_id: 'channel' })

    const onvif = buildDevicePayload(
      {
        ...defaults,
        name: 'ONVIF',
        input_type: 'onvif',
        onvif_host: 'camera',
        onvif_port: 8899,
        onvif_profile_token: 'main',
      },
      options,
    )
    expect(JSON.parse(onvif.input_value)).toMatchObject({
      host: 'camera',
      port: 8899,
      profile_token: 'main',
    })
  })
})

describe('input type settings', () => {
  test('maps every selectable input type to a settings component', () => {
    const kinds = Object.fromEntries(
      inputTypeOptions.map((option) => [option.value, inputSettingsKind(option.value)]),
    )
    expect(kinds).toEqual({
      rtsp: 'url',
      rtmp: 'url',
      stream: 'url',
      file: 'url',
      v4l2: 'url',
      x11grab: 'url',
      lavfi: 'url',
      xiaomi: 'xiaomi',
      gb28181: 'gb28181',
      onvif: 'onvif',
    })
  })

  test('gives every address-based input type its own label and example', () => {
    const urlTypes = inputTypeOptions
      .map((option) => option.value)
      .filter((value) => inputSettingsKind(value) === 'url')
    const labels = new Set(urlTypes.map((value) => urlInputMeta(value).label))
    expect(labels.size).toBe(urlTypes.length)
    expect(urlInputMeta('stream').hint).toContain('yt-dlp')
    expect(urlInputMeta('unknown').label).toBe('输入地址/标识')
  })
})
