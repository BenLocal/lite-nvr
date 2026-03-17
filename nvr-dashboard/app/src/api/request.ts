import { getAuthToken } from '../auth/token'

type RequestMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

interface RequestOptions extends Omit<RequestInit, 'method' | 'body'> {
  method?: RequestMethod
  body?: unknown
}

interface BaseResponse<T> {
  code: number
  message: string
  data: T | null
}

const API_BASE = import.meta.env.DEV ? '/api' : ''

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
