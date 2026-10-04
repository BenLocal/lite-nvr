import { request } from './request'

export interface TransportConfig {
  host: string
  port?: number
  share?: string
  workgroup?: string
  username: string
  password: string
  base_path: string
  stream_ids?: string[] | null
}

export interface TransportTarget {
  id: string
  name: string
  kind: 'ftp' | 'smb'
  enabled: boolean
  config: TransportConfig
  remark: string
  done: number
  failed: number
}

export type TransportPayload = Pick<TransportTarget, 'name' | 'kind' | 'enabled' | 'config' | 'remark'>

function isTarget(value: unknown): value is TransportTarget {
  if (typeof value !== 'object' || value === null) return false
  if (!('id' in value && typeof value.id === 'string'
    && 'name' in value && typeof value.name === 'string'
    && 'kind' in value && (value.kind === 'ftp' || value.kind === 'smb')
    && 'enabled' in value && typeof value.enabled === 'boolean'
    && 'remark' in value && typeof value.remark === 'string'
    && 'done' in value && typeof value.done === 'number'
    && 'failed' in value && typeof value.failed === 'number'
    && 'config' in value && typeof value.config === 'object' && value.config !== null)) return false
  const config = value.config
  return 'host' in config && typeof config.host === 'string'
    && 'username' in config && typeof config.username === 'string'
    && 'password' in config && typeof config.password === 'string'
    && 'base_path' in config && typeof config.base_path === 'string'
    && (!('port' in config) || typeof config.port === 'number')
    && (!('share' in config) || typeof config.share === 'string')
    && (!('workgroup' in config) || typeof config.workgroup === 'string')
    && (!('stream_ids' in config) || config.stream_ids === null
      || (Array.isArray(config.stream_ids) && config.stream_ids.every((id: unknown) => typeof id === 'string')))
}

export async function listTransportTargets(): Promise<TransportTarget[]> {
  const data = await request<unknown>('/transport/targets')
  if (!Array.isArray(data)) throw new Error('转存目标配置格式错误')
  return data.map((value: unknown) => {
    if (typeof value !== 'object' || value === null || !('config' in value)
      || typeof value.config !== 'object' || value.config === null) throw new Error('转存目标配置格式错误')
    const target = { ...value, config: { username: '', password: '', base_path: '', ...value.config } }
    if (!isTarget(target)) throw new Error('转存目标配置格式错误')
    return target
  })
}

export async function getTransportCapabilities(): Promise<{ smb_enabled: boolean }> {
  const data = await request<unknown>('/transport/capabilities')
  if (typeof data !== 'object' || data === null || !('smb_enabled' in data)
    || typeof data.smb_enabled !== 'boolean') throw new Error('无法读取转存能力')
  return { smb_enabled: data.smb_enabled }
}

export async function saveTransportTarget(id: string | null, payload: TransportPayload): Promise<void> {
  await request(id ? `/transport/target/update/${encodeURIComponent(id)}` : '/transport/target/add', {
    method: 'POST', body: payload,
  })
}

export async function removeTransportTarget(id: string): Promise<void> {
  await request(`/transport/target/remove/${encodeURIComponent(id)}`, { method: 'POST' })
}

export async function testTransportTarget(id: string): Promise<void> {
  await request(`/transport/target/test/${encodeURIComponent(id)}`, { method: 'POST' })
}
