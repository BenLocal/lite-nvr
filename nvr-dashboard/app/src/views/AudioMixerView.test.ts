import { mount, flushPromises } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import PrimeVue from 'primevue/config'
import AudioMixerView from './AudioMixerView.vue'
import { listDevices } from '../api/device'
import { getAudioSources, getMixer, getProgramConfig } from '../api/audiomixer'

vi.mock('../api/device', () => ({ listDevices: vi.fn() }))
vi.mock('../api/audiomixer', async (importOriginal) => ({
  ...await importOriginal<typeof import('../api/audiomixer')>(),
  getAudioSources: vi.fn(), getMixer: vi.fn(), getProgramConfig: vi.fn(),
}))
vi.mock('../utils/streamPlayer', () => ({ createStreamPlayer: vi.fn() }))
vi.mock('../utils/toast', () => ({ useAppToast: () => ({ success: vi.fn(), error: vi.fn(), errorFrom: vi.fn() }) }))

describe('mixer device list', () => {
  it('shows all three devices, including a device without original audio and an opted-out output', async () => {
    vi.mocked(listDevices).mockResolvedValue([
      { id: 'one', name: 'Camera One', input_type: 'rtsp', input_value: '', description: '', include_audio: true, record: false, created_at: '', updated_at: '' },
      { id: 'two', name: 'Camera Two', input_type: 'v4l2', input_value: '', description: '', include_audio: false, record: false, created_at: '', updated_at: '' },
      { id: 'three', name: 'Camera Three', input_type: 'onvif', input_value: '', description: '', include_audio: true, record: false, config: { output: { use_global_audio: false } }, created_at: '', updated_at: '' },
    ])
    vi.mocked(getAudioSources).mockResolvedValue([{ id: 'one', name: 'Camera One', kind: 'device' }])
    vi.mocked(getMixer).mockResolvedValue({ sources: [], buses: [{ id: 'main', publish_url: '', flv_url: '', inputs: [{ source_id: 'one', volume: 100, muted: false }] }] })
    vi.mocked(getProgramConfig).mockResolvedValue({
      config: { enabled: true, mode: 'program', active_bus_id: 'main', profile: { codec: 'aac', sample_rate: 48000, channels: 2, bitrate: 128000 }, version: 1 },
      outputs: [
        { device_id: 'one', use_global_audio: true, effective_version: 1, state: 'applied' },
        { device_id: 'two', use_global_audio: true, effective_version: null, state: 'waiting' },
        { device_id: 'three', use_global_audio: false, effective_version: null, state: 'original' },
      ],
    })
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
    vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })))
    const wrapper = mount(AudioMixerView, { global: { plugins: [PrimeVue], stubs: { teleport: true } } })
    try {
      await flushPromises()
      const list = wrapper.get('.device-list-card')
      expect(list.text()).toContain('设备列表（3）')
      const rows = list.findAll('tbody tr')
      expect(rows).toHaveLength(3)
      expect(rows[0]?.text()).toContain('Camera One')
      expect(rows[0]?.text()).toContain('main')
      expect(rows[0]?.text()).toContain('已接入节目')
      expect(rows[1]?.text()).toContain('Camera Two')
      expect(rows[1]?.text()).toContain('忽略')
      expect(rows[1]?.text()).toContain('等待连接')
      expect(rows[2]?.text()).toContain('Camera Three')
      expect(rows[2]?.text()).toContain('关闭')
      expect(rows[2]?.text()).toContain('原音路径')
    } finally {
      wrapper.unmount()
      vi.restoreAllMocks()
      vi.unstubAllGlobals()
    }
  })
})
