import { getAuthToken } from '../auth/token'

// The ASR control endpoints return a short plain-text status ("started" /
// "stopped" / "no audio: ..."), not the shared `{ code, message, data }`
// envelope, so they use a dedicated raw-fetch helper instead of `request()`.
async function post(path: string): Promise<string> {
  const token = getAuthToken()
  const headers = new Headers()
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  const res = await fetch(`/api${path}`, { method: 'POST', headers })
  const text = await res.text().catch(() => '')
  if (!res.ok) {
    throw new Error(text || `请求失败 (${res.status})`)
  }
  return text
}

/** Begin transcribing the given pipe's live audio. */
export function startAsr(pipe: string): Promise<string> {
  return post(`/asr/${encodeURIComponent(pipe)}/start`)
}

/** Stop transcribing the given pipe. */
export function stopAsr(pipe: string): Promise<string> {
  return post(`/asr/${encodeURIComponent(pipe)}/stop`)
}
