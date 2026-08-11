import type { DetectConfig, DevicePayload } from '../api/device'

export interface DetectionFormValues extends Record<string, unknown> {
  detect_enabled: boolean
  detect_models: string[]
  detect_sample_every_ms: number
  detect_min_confidence: number
}

export type FormErrors = Record<string, { message: string }[]>

export function detectionFormValues(config?: DetectConfig): DetectionFormValues {
  return {
    detect_enabled: config?.enabled ?? false,
    detect_models: [...(config?.models ?? [])],
    detect_sample_every_ms: config?.sample_every_ms ?? 1_000,
    detect_min_confidence: config?.min_confidence ?? 0,
  }
}

export function validateDetectionValues(
  values: Record<string, unknown>,
  maxSampleIntervalMs: number,
): FormErrors {
  const errors: FormErrors = {}
  const interval = Number(values.detect_sample_every_ms)
  if (!Number.isInteger(interval) || interval < 0 || interval > maxSampleIntervalMs) {
    errors.detect_sample_every_ms = [
      { message: `抽帧间隔必须是 0 到 ${maxSampleIntervalMs} 之间的整数` },
    ]
  }
  const confidence = Number(values.detect_min_confidence)
  if (!Number.isFinite(confidence) || confidence < 0 || confidence > 1) {
    errors.detect_min_confidence = [{ message: '置信度下限必须是 0 到 1 之间的数值' }]
  }
  return errors
}

export function buildDetectionConfig(values: Record<string, unknown>): DetectConfig {
  const models = Array.isArray(values.detect_models)
    ? values.detect_models.filter((name): name is string => typeof name === 'string')
    : []
  return {
    enabled: Boolean(values.detect_enabled),
    models: [...models],
    sample_every_ms: Number(values.detect_sample_every_ms),
    min_confidence: Number(values.detect_min_confidence),
  }
}

export function withDetectionConfig(
  payload: DevicePayload,
  values: Record<string, unknown>,
  supportedInputTypes: readonly string[],
): DevicePayload {
  if (!supportedInputTypes.includes(payload.input_type)) {
    return { ...payload, config: undefined }
  }
  return {
    ...payload,
    config: {
      ...payload.config,
      detect: buildDetectionConfig(values),
    },
  }
}
