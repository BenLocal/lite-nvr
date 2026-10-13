import { describe, expect, it } from 'vitest'
import { parseProgramState } from './audiomixer'

const config = { enabled: true, mode: 'program', active_bus_id: 'main', profile: { codec: 'aac', sample_rate: 48000, channels: 2, bitrate: 128000 }, version: 1 }

describe('mixer program contract', () => {
  it('distinguishes a saved setting from an output actually applying it', () => {
    const value = { config, outputs: [{ device_id: 'camera', use_global_audio: true, effective_version: null, state: 'waiting' }] }
    expect(parseProgramState(value)).toEqual(value)
  })
  it('rejects invalid audio formats and incomplete output state', () => {
    expect(() => parseProgramState({ config: { ...config, profile: { ...config.profile, channels: 1 } }, outputs: [] })).toThrow()
    expect(() => parseProgramState({ config, outputs: [{}] })).toThrow()
  })
})
