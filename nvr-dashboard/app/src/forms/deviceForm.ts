import type { DeviceItem, DevicePayload } from '../api/device'
import {
  detectionFormValues,
  validateDetectionValues,
  withDetectionConfig,
  type FormErrors,
} from './detectionForm'

export const inputTypeOptions = [
  { label: 'RTSP', value: 'rtsp', icon: 'pi pi-video' },
  { label: 'RTMP', value: 'rtmp', icon: 'pi pi-upload' },
  { label: '平台直播', value: 'stream', icon: 'pi pi-globe' },
  { label: '文件', value: 'file', icon: 'pi pi-file' },
  { label: 'V4L2', value: 'v4l2', icon: 'pi pi-camera' },
  { label: 'X11 Grab', value: 'x11grab', icon: 'pi pi-desktop' },
  { label: 'Lavfi', value: 'lavfi', icon: 'pi pi-palette' },
  { label: '小米摄像头', value: 'xiaomi', icon: 'pi pi-mobile' },
  { label: '国标 GB28181', value: 'gb28181', icon: 'pi pi-sitemap' },
  { label: 'ONVIF 摄像头', value: 'onvif', icon: 'pi pi-wifi' },
]

/**
 * The `$form` slot of @primevue/forms: each field's reactive state. Writing
 * `.value` sets the field, as `setFieldValue` does internally.
 */
export type DeviceFormFields = Record<
  string,
  { value?: unknown; invalid?: boolean; error?: { message?: string } } | undefined
>

/** Which settings component an input type uses. */
export type InputSettingsKind = 'url' | 'v4l2' | 'x11grab' | 'xiaomi' | 'gb28181' | 'onvif'

export function inputSettingsKind(inputType: string): InputSettingsKind {
  switch (inputType) {
    case 'v4l2':
    case 'x11grab':
    case 'xiaomi':
    case 'gb28181':
    case 'onvif':
      return inputType
  }
  return 'url'
}

export interface UrlInputMeta {
  label: string
  placeholder: string
  hint?: string
}

const URL_INPUT_META: Record<string, UrlInputMeta> = {
  rtsp: { label: 'RTSP 地址', placeholder: 'rtsp://user:pass@192.168.1.10:554/stream1' },
  rtmp: { label: 'RTMP 地址', placeholder: 'rtmp://host/live/stream' },
  stream: {
    label: '直播间地址',
    placeholder: '如 https://www.twitch.tv/xxx 或 https://live.bilibili.com/123',
    hint:
      '填直播间页面地址（B站/虎牙/斗鱼/Twitch/YouTube 等）。拉流地址由 yt-dlp ' +
      '在启动和每次重连时自动重新解析（地址带签名会过期），服务器需已安装 yt-dlp。',
  },
  file: { label: '文件路径', placeholder: '/data/videos/demo.mp4', hint: '服务器上的本地媒体文件路径' },
  v4l2: {
    label: '设备节点',
    placeholder: '/dev/video0',
    hint: '下拉列出 nvr 上可采集视频的 V4L2 节点，也可直接输入路径',
  },
  x11grab: {
    label: '显示器',
    placeholder: ':0',
    hint: '下拉列出 nvr 上正在运行的 X11 显示器，也可直接输入（如 :0 或 :0.0）',
  },
  lavfi: {
    label: '滤镜图',
    placeholder: 'testsrc=size=1280x720:rate=25',
    hint: 'FFmpeg lavfi 滤镜描述，常用于生成测试图案',
  },
}

/** Label, example and hint for an input type that takes a single address. */
export function urlInputMeta(inputType: string): UrlInputMeta {
  return URL_INPUT_META[inputType] ?? { label: '输入地址/标识', placeholder: '如 rtsp://camera/live' }
}

// go2rtc Xiaomi cloud regions ("" = mainland China).
export const xiaomiRegionOptions = [
  { label: '中国大陆', value: '' },
  { label: '德国 (de)', value: 'de' },
  { label: '印度 (i2)', value: 'i2' },
  { label: '俄罗斯 (ru)', value: 'ru' },
  { label: '新加坡 (sg)', value: 'sg' },
  { label: '美国 (us)', value: 'us' },
]

interface DeviceFormValues extends Record<string, unknown> {
  name: string
  input_type: string
  input_value: string
  description: string
  include_audio: boolean
  record: boolean
}

interface ResolveOptions {
  gbDeviceId: string
  gbChannelId: string
  validateDetection: boolean
  maxDetectionSampleIntervalMs: number
}

interface PayloadOptions {
  gbDeviceId: string
  gbChannelId: string
  supportedDetectionInputTypes: readonly string[]
}

interface ResolvedFields {
  values: Record<string, unknown>
  errors: FormErrors
}

const DEFAULT_FORM_VALUES: DeviceFormValues = {
  name: '',
  input_type: 'rtsp',
  input_value: '',
  description: '',
  include_audio: false,
  record: true,
  ...detectionFormValues(),
  xm_user_id: '',
  xm_token: '',
  xm_region: '',
  xm_did: '',
  xm_model: '',
  xm_ip: '',
  onvif_host: '',
  onvif_port: 80,
  onvif_username: '',
  onvif_password: '',
  onvif_profile_token: '',
}

export function deviceFormInitialValues(device: DeviceItem | null): DeviceFormValues {
  if (!device) {
    return { ...DEFAULT_FORM_VALUES, ...detectionFormValues() }
  }
  const base: DeviceFormValues = {
    ...DEFAULT_FORM_VALUES,
    name: device.name,
    input_type: device.input_type,
    input_value: device.input_value,
    description: device.description,
    include_audio: device.include_audio,
    record: device.record,
    ...detectionFormValues(device.config?.detect),
  }
  if (device.input_type === 'xiaomi') return withXiaomiInitialValues(base)
  if (device.input_type === 'onvif') return withOnvifInitialValues(base)
  return base
}

function withXiaomiInitialValues(base: DeviceFormValues): DeviceFormValues {
  try {
    const config = JSON.parse(base.input_value) as Partial<
      Record<'user_id' | 'token' | 'region' | 'did' | 'model' | 'ip', string>
    >
    return {
      ...base,
      xm_user_id: config.user_id ?? '',
      xm_token: config.token ?? '',
      xm_region: config.region ?? '',
      xm_did: config.did ?? '',
      xm_model: config.model ?? '',
      xm_ip: config.ip ?? '',
    }
  } catch {
    return base
  }
}

function withOnvifInitialValues(base: DeviceFormValues): DeviceFormValues {
  try {
    const config = JSON.parse(base.input_value) as Partial<{
      host: string
      port: number
      username: string
      password: string
      profile_token: string
    }>
    return {
      ...base,
      onvif_host: config.host ?? '',
      onvif_port: config.port ?? 80,
      onvif_username: config.username ?? '',
      onvif_password: config.password ?? '',
      onvif_profile_token: config.profile_token ?? '',
    }
  } catch {
    return base
  }
}

export function resolveDeviceForm(
  { values }: { values: Record<string, unknown> },
  options: ResolveOptions,
) {
  const base = resolveBaseFields(values)
  const input = resolveInputFields(values, String(base.values.input_type), options)
  const detectionErrors = options.validateDetection
    ? validateDetectionValues(values, options.maxDetectionSampleIntervalMs)
    : {}
  return {
    values: { ...base.values, ...input.values },
    errors: { ...base.errors, ...detectionErrors, ...input.errors },
  }
}

function resolveBaseFields(values: Record<string, unknown>): ResolvedFields {
  const name = String(values.name ?? '').trim()
  const inputType = String(values.input_type ?? '').trim()
  const errors: FormErrors = {
    ...(!name ? { name: [{ message: '请输入设备名称' }] } : {}),
    ...(!inputType ? { input_type: [{ message: '请选择输入类型' }] } : {}),
  }
  return {
    values: {
      name,
      input_type: inputType,
      description: String(values.description ?? '').trim(),
      include_audio: Boolean(values.include_audio),
      record: values.record === undefined ? true : Boolean(values.record),
      detect_enabled: Boolean(values.detect_enabled),
      detect_models: Array.isArray(values.detect_models) ? [...values.detect_models] : [],
      detect_sample_every_ms: Number(values.detect_sample_every_ms),
      detect_min_confidence: Number(values.detect_min_confidence),
    },
    errors,
  }
}

function resolveInputFields(
  values: Record<string, unknown>,
  inputType: string,
  options: ResolveOptions,
): ResolvedFields {
  if (inputType === 'xiaomi') return resolveXiaomiFields(values)
  if (inputType === 'gb28181') return resolveGbFields(options)
  if (inputType === 'onvif') return resolveOnvifFields(values)
  const inputValue = String(values.input_value ?? '').trim()
  return {
    values: { input_value: inputValue },
    errors: inputValue ? {} : { input_value: [{ message: '请输入输入地址或标识' }] },
  }
}

function resolveXiaomiFields(values: Record<string, unknown>): ResolvedFields {
  const config = {
    user_id: String(values.xm_user_id ?? '').trim(),
    token: String(values.xm_token ?? '').trim(),
    region: String(values.xm_region ?? '').trim(),
    did: String(values.xm_did ?? '').trim(),
    model: String(values.xm_model ?? '').trim(),
    ip: String(values.xm_ip ?? '').trim(),
  }
  return {
    values: {
      xm_user_id: config.user_id,
      xm_token: config.token,
      xm_region: config.region,
      xm_did: config.did,
      xm_model: config.model,
      xm_ip: config.ip,
      input_value: JSON.stringify(config),
    },
    errors: {
      ...(!config.user_id ? { xm_user_id: [{ message: '请输入用户 ID' }] } : {}),
      ...(!config.token ? { xm_token: [{ message: '请输入 Token' }] } : {}),
      ...(!config.did ? { xm_did: [{ message: '请输入设备 DID' }] } : {}),
      ...(!config.model ? { xm_model: [{ message: '请输入设备型号' }] } : {}),
      ...(!config.ip ? { xm_ip: [{ message: '请输入摄像头 IP' }] } : {}),
    },
  }
}

function resolveGbFields(options: ResolveOptions): ResolvedFields {
  const message = !options.gbDeviceId
    ? '请选择国标设备'
    : !options.gbChannelId
      ? '请选择国标通道'
      : ''
  return {
    values: {
      input_value: JSON.stringify({
        device_id: options.gbDeviceId,
        channel_id: options.gbChannelId,
      }),
    },
    errors: message ? { input_value: [{ message }] } : {},
  }
}

function resolveOnvifFields(values: Record<string, unknown>): ResolvedFields {
  const config = {
    host: String(values.onvif_host ?? '').trim(),
    port: Number(values.onvif_port ?? 80),
    username: String(values.onvif_username ?? '').trim(),
    password: String(values.onvif_password ?? '').trim(),
    profile_token: String(values.onvif_profile_token ?? '').trim(),
  }
  return {
    values: {
      onvif_host: config.host,
      onvif_port: config.port,
      onvif_username: config.username,
      onvif_password: config.password,
      onvif_profile_token: config.profile_token,
      input_value: JSON.stringify(config),
    },
    errors: {
      ...(!config.host ? { onvif_host: [{ message: '请输入主机地址' }] } : {}),
      ...(!config.profile_token
        ? { input_value: [{ message: '请先探测设备并选择视频配置文件' }] }
        : {}),
    },
  }
}

export function buildDevicePayload(
  values: Record<string, unknown>,
  options: PayloadOptions,
): DevicePayload {
  const inputType = String(values.input_type ?? '')
  const basePayload: DevicePayload = {
    name: String(values.name ?? ''),
    input_type: inputType,
    input_value: serializeInputValue(values, inputType, options),
    description: String(values.description ?? ''),
    include_audio: Boolean(values.include_audio),
    record: values.record === undefined ? true : Boolean(values.record),
  }
  return withDetectionConfig(
    basePayload,
    values,
    options.supportedDetectionInputTypes,
  )
}

function serializeInputValue(
  values: Record<string, unknown>,
  inputType: string,
  options: PayloadOptions,
): string {
  if (inputType === 'xiaomi') {
    return JSON.stringify({
      user_id: String(values.xm_user_id ?? '').trim(),
      token: String(values.xm_token ?? '').trim(),
      region: String(values.xm_region ?? '').trim(),
      did: String(values.xm_did ?? '').trim(),
      model: String(values.xm_model ?? '').trim(),
      ip: String(values.xm_ip ?? '').trim(),
    })
  }
  if (inputType === 'gb28181') {
    return JSON.stringify({ device_id: options.gbDeviceId, channel_id: options.gbChannelId })
  }
  if (inputType === 'onvif') {
    return JSON.stringify({
      host: String(values.onvif_host ?? '').trim(),
      port: Number(values.onvif_port ?? 80),
      username: String(values.onvif_username ?? '').trim(),
      password: String(values.onvif_password ?? '').trim(),
      profile_token: String(values.onvif_profile_token ?? '').trim(),
    })
  }
  return String(values.input_value ?? '')
}
