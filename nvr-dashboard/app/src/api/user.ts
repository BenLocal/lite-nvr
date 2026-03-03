import { request } from './request'

export interface LoginRequest {
  username: string
  password: string
}

export interface LoginResponse {
  token: string
}

export function loginByPassword(payload: LoginRequest) {
  return request<LoginResponse>('/user/login', {
    method: 'POST',
    body: payload,
  })
}
