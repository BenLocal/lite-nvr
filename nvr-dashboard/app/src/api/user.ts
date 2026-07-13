import { request } from './request'

export interface LoginRequest {
  username: string
  password: string
}

export interface LoginResponse {
  token: string
  username: string
}

export function loginByPassword(payload: LoginRequest) {
  return request<LoginResponse>('/user/login', {
    method: 'POST',
    body: payload,
  })
}

export function logout() {
  return request<null>('/user/logout', { method: 'POST' })
}

export interface UserInfo {
  username: string
}

export function getUserInfo() {
  return request<UserInfo>('/user/info')
}

export interface ChangePasswordRequest {
  old_password: string
  new_password: string
}

export function changePassword(payload: ChangePasswordRequest) {
  return request<null>('/user/password', {
    method: 'POST',
    body: payload,
  })
}

export interface UserListItem {
  username: string
  create_time: string
  update_time: string
}

export function listUsers() {
  return request<UserListItem[]>('/user/list')
}

export interface AddUserRequest {
  username: string
  password: string
}

export function addUser(payload: AddUserRequest) {
  return request<null>('/user/add', {
    method: 'POST',
    body: payload,
  })
}

export function removeUser(username: string) {
  return request<null>(`/user/remove/${encodeURIComponent(username)}`, {
    method: 'POST',
  })
}
