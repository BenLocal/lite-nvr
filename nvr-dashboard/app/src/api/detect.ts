import { getAuthToken } from '../auth/token'

// Detection control + read endpoints return plain text (start/stop) or bare
// JSON (latest/models), not the shared `{ code, message, data }` envelope, so
// they use raw fetch — same pattern as `src/api/asr.ts`.

export interface BBox {
  x1: number
  y1: number
  x2: number
  y2: number
}

export interface Detection {
  class_id: number
  label: string
  bbox: BBox
  confidence: number
}

export interface ModelResult {
  name: string
  infer_ms: number
  error: string | null
  detections: Detection[]
}

export interface FrameResult {
  ts: number
  frame_w: number
  frame_h: number
  models: ModelResult[]
}

export interface DetectionCapabilities {
  models: string[]
  supported_input_types: string[]
  max_sample_interval_ms: number
  max_model_count: number
  max_model_name_chars: number
}

export type DetectStartResult =
  | { status: 'started'; lease: string }
  | { status: 'already running' }

function authHeaders(): Headers {
  const token = getAuthToken()
  const headers = new Headers()
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  return headers
}

/** Start detection for a pipe (all configured models run). Status string. */
export async function startDetect(pipe: string): Promise<DetectStartResult> {
  const res = await fetch(`/api/detect/${encodeURIComponent(pipe)}/start`, {
    method: 'POST',
    headers: authHeaders(),
  })
  const text = await res.text().catch(() => '')
  if (!res.ok) {
    throw new Error(text || `请求失败 (${res.status})`)
  }
  if (text === 'already running') {
    return { status: text }
  }
  if (text !== 'started') {
    throw new Error(`未知检测启动状态: ${text || '(empty)'}`)
  }
  const lease = res.headers.get('x-detection-tap-lease')
  if (!lease) {
    throw new Error('检测启动响应缺少 tap lease')
  }
  return { status: text, lease }
}

/** Stop detection for a pipe. Status string. */
export async function stopDetect(pipe: string, lease?: string): Promise<string> {
  const headers = authHeaders()
  if (lease) headers.set('Content-Type', 'application/json')
  const res = await fetch(`/api/detect/${encodeURIComponent(pipe)}/stop`, {
    method: 'POST',
    headers,
    body: lease ? JSON.stringify({ lease }) : undefined,
  })
  const text = await res.text().catch(() => '')
  if (!res.ok) {
    throw new Error(text || `请求失败 (${res.status})`)
  }
  return text
}

/** Latest per-frame, multi-model result — or `null` if none yet (HTTP 404). */
export async function getDetectLatest(pipe: string): Promise<FrameResult | null> {
  const res = await fetch(`/api/detect/${encodeURIComponent(pipe)}/latest`, {
    headers: authHeaders(),
  })
  if (res.status === 404) {
    return null
  }
  if (!res.ok) {
    throw new Error(`请求失败 (${res.status})`)
  }
  return (await res.json()) as FrameResult
}

/** Names of the configured detection models. */
export async function listDetectModels(): Promise<string[]> {
  const res = await fetch('/api/detect/models', { headers: authHeaders() })
  if (!res.ok) {
    throw new Error(`请求失败 (${res.status})`)
  }
  return (await res.json()) as string[]
}

/** Detection form contract, sourced from the backend. */
export async function getDetectionCapabilities(): Promise<DetectionCapabilities> {
  const res = await fetch('/api/detect/capabilities', { headers: authHeaders() })
  if (!res.ok) {
    throw new Error(`请求失败 (${res.status})`)
  }
  return (await res.json()) as DetectionCapabilities
}
