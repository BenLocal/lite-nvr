import { clearAuthToken, getAuthToken } from '../auth/token'

// Project convention: REST API uses only GET and POST (no PUT/PATCH/DELETE).
type RequestMethod = 'GET' | 'POST'

const API_BASE = '/api'

interface RequestOptions extends Omit<RequestInit, 'method' | 'body'> {
  method?: RequestMethod
  body?: unknown
}

interface BaseResponse<T> {
  code: number
  message: string
  data: T | null
}


export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = 'GET', body, headers, ...rest } = options
  const token = getAuthToken()
  const requestHeaders = new Headers(headers)
  requestHeaders.set('Content-Type', 'application/json')
  if (token) {
    requestHeaders.set('Authorization', `Bearer ${token}`)
  }

  const response = await fetch(`${API_BASE}${path}`, {
    method,
    headers: requestHeaders,
    body: body === undefined ? undefined : JSON.stringify(body),
    ...rest,
  })

  if (response.status === 401) {
    // Session missing/expired/revoked: drop local auth and return to login.
    clearAuthToken()
    const loginUrl = `${import.meta.env.BASE_URL}login`
    if (!window.location.pathname.startsWith(loginUrl)) {
      window.location.assign(loginUrl)
    }
    throw new Error('登录已过期，请重新登录')
  }

  let payload: BaseResponse<T>
  try {
    payload = (await response.json()) as BaseResponse<T>
  } catch {
    throw new Error(`Request failed with status ${response.status}`)
  }

  if (!response.ok || payload.code !== 0) {
    throw new Error(payload.message || `Request failed with status ${response.status}`)
  }

  return payload.data as T
}
