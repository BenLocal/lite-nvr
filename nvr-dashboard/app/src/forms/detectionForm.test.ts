import { describe, expect, it } from 'vitest'
import type { DevicePayload } from '../api/device'
import {
  buildDetectionConfig,
  detectionFormValues,
  validateDetectionValues,
  withDetectionConfig,
} from './detectionForm'

describe('detection form contract', () => {
  it('hydrates defaults and copies persisted model names', () => {
    const models = ['person']
    const values = detectionFormValues({
      enabled: true,
      models,
      sample_every_ms: 0,
      min_confidence: 0.4,
    })

    expect(values).toEqual({
      detect_enabled: true,
      detect_models: ['person'],
      detect_sample_every_ms: 0,
      detect_min_confidence: 0.4,
    })
    expect(values.detect_models).not.toBe(models)
    expect(detectionFormValues()).toEqual({
      detect_enabled: false,
      detect_models: [],
      detect_sample_every_ms: 1_000,
      detect_min_confidence: 0,
    })
  })

  it('validates interval and confidence at the form boundary', () => {
    expect(
      validateDetectionValues(
        { detect_sample_every_ms: 3_600_001, detect_min_confidence: 1.1 },
        3_600_000,
      ),
    ).toEqual({
      detect_sample_every_ms: [{ message: '抽帧间隔必须是 0 到 3600000 之间的整数' }],
      detect_min_confidence: [{ message: '置信度下限必须是 0 到 1 之间的数值' }],
    })
    expect(
      validateDetectionValues(
        { detect_sample_every_ms: Number.NaN, detect_min_confidence: Number.NaN },
        3_600_000,
      ),
    ).toHaveProperty('detect_sample_every_ms')
    expect(
      validateDetectionValues(
        { detect_sample_every_ms: 0, detect_min_confidence: 1 },
        3_600_000,
      ),
    ).toEqual({})
  })

  it('builds a copied config and a new payload without mutating inputs', () => {
    const selectedModels = ['person']
    const values = {
      detect_enabled: true,
      detect_models: selectedModels,
      detect_sample_every_ms: 2_000,
      detect_min_confidence: 0.5,
    }
    const payload: DevicePayload = {
      name: 'cam',
      input_type: 'rtsp',
      input_value: 'rtsp://example/live',
      config: {},
    }

    const config = buildDetectionConfig(values)
    const next = withDetectionConfig(payload, values, ['rtsp'])

    expect(config.models).toEqual(['person'])
    expect(config.models).not.toBe(selectedModels)
    expect(next).not.toBe(payload)
    expect(next.config).not.toBe(payload.config)
    expect(next.config?.detect).toEqual(config)
    expect(payload.config).toEqual({})
  })
})
