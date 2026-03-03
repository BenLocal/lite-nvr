import { getAuthToken } from '../auth/token'

type RequestMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

interface RequestOptions extends Omit<RequestInit, 'method' | 'body'> {
  method?: RequestMethod
  body?: unknown
}

const API_BASE = import.meta.env.DEV ? '/api' : ''

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = 'GET', body, headers, ...rest } = options
  const token = getAuthToken()

  const authHeaders = token ? { Authorization: `Bearer ${token}` } : {}
  const response = await fetch(`${API_BASE}${path}`, {
    method,
    headers: {
      'Content-Type': 'application/json',
      ...authHeaders,
      ...headers,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    ...rest,
  })

  if (!response.ok) {
    let message = `Request failed with status ${response.status}`
    try {
      const text = await response.text()
      if (text) {
        message = text
      }
    } catch {
      // ignore non-text response parse failures
    }
    throw new Error(message)
  }

  return response.json() as Promise<T>
}
