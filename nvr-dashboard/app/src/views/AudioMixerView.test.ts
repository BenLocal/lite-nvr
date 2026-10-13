import { mount, flushPromises } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import PrimeVue from 'primevue/config'
import AudioMixerView from './AudioMixerView.vue'
import { listDevices } from '../api/device'
import { getAudioSources, getMixer, startBus, stopBus } from '../api/audiomixer'

vi.mock('../api/device', () => ({ listDevices: vi.fn() }))
vi.mock('../api/audiomixer', async (importOriginal) => ({
  ...await importOriginal<typeof import('../api/audiomixer')>(),
  getAudioSources: vi.fn(), getMixer: vi.fn(), startBus: vi.fn(), stopBus: vi.fn(),
}))
vi.mock('../utils/streamPlayer', () => ({ createStreamPlayer: vi.fn() }))
vi.mock('../utils/toast', () => ({ useAppToast: () => ({ success: vi.fn(), error: vi.fn(), errorFrom: vi.fn() }) }))

describe('mixer physical input list', () => {
  it('shows the terminal and both HDMI capture nodes independently of camera devices', async () => {
    vi.mocked(listDevices).mockResolvedValue([
      { id: 'one', name: 'Camera One', input_type: 'rtsp', input_value: '', description: '', include_audio: true, record: false, created_at: '', updated_at: '' },
      { id: 'two', name: 'Camera Two', input_type: 'v4l2', input_value: '', description: '', include_audio: false, record: false, created_at: '', updated_at: '' },
      { id: 'three', name: 'Camera Three', input_type: 'onvif', input_value: '', description: '', include_audio: true, record: false, config: { output: { use_global_audio: false } }, created_at: '', updated_at: '' },
    ])
    vi.mocked(getAudioSources).mockResolvedValue([
      { id: 'one', name: 'Camera One', kind: 'device' },
      { id: 'alsa:rockchipes8388', name: '端子音频（左右双声道）', kind: 'alsa' },
      { id: 'alsa:D6911codec', name: 'HDMI 1 音频', kind: 'alsa' },
      { id: 'alsa:rockchiphdmiin', name: 'HDMI 2 音频', kind: 'alsa' },
    ])
    vi.mocked(getMixer).mockResolvedValue({
      sources: [
        { id: 'alsa:rockchipes8388', url: '', ready: true },
        { id: 'alsa:D6911codec', url: '', ready: false, error: '等待 HDMI 信号' },
      ],
      buses: [{ enabled: true, profile: { codec: 'aac', sample_rate: 48000, channels: 2, bitrate: 128000 }, id: 'main', publish_url: '', flv_url: '', inputs: [{ source_id: 'alsa:rockchipes8388', volume: 100, muted: false }] }],
    })
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
    vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })))
    const wrapper = mount(AudioMixerView, { global: { plugins: [PrimeVue], stubs: { teleport: true } } })
    try {
      await flushPromises()
      const list = wrapper.get('.physical-input-card')
      expect(list.text()).toContain('音频输入节点（3）')
      const rows = list.findAll('tbody tr')
      expect(rows).toHaveLength(3)
      expect(rows[0]?.text()).toContain('端子音频')
      expect(rows[0]?.get('input[type=checkbox]').element).toHaveProperty('checked', true)
      expect(rows[0]?.text()).toContain('已收到音频')
      expect(rows[1]?.text()).toContain('HDMI 1 音频')
      expect(rows[1]?.text()).toContain('等待信号')
      expect(rows[1]?.text()).toContain('等待 HDMI 信号')
      expect(rows[2]?.text()).toContain('HDMI 2 音频')
      expect(rows[2]?.text()).toContain('未接入')
      expect(list.text()).not.toContain('Camera One')
      expect(wrapper.text()).not.toContain('设备输出节目音频')
      expect(wrapper.find('.program-card').exists()).toBe(false)
      expect(wrapper.find('.bus-card').exists()).toBe(false)
      expect(list.text()).toContain('节目采样率')
      expect(wrapper.findAll('.page-actions button')).toHaveLength(2)
      expect(wrapper.get('.page-actions button:first-child').text()).toContain('停止')
      expect(wrapper.text()).not.toContain('新建输出总线')
      expect(wrapper.findAll('input[type=checkbox]').length).toBeGreaterThanOrEqual(3)
      const active = await getMixer()
      const inactive = { ...active, sources: [], buses: active.buses.map(bus => ({ ...bus, enabled: false })) }
      vi.mocked(stopBus).mockResolvedValue(inactive)
      await wrapper.get('.page-actions button:first-child').trigger('click')
      await flushPromises()
      expect(stopBus).toHaveBeenCalledWith('main')
      expect(wrapper.get('.page-actions button:first-child').text()).toContain('开始')
      vi.mocked(startBus).mockResolvedValue(active)
      await wrapper.get('.page-actions button:first-child').trigger('click')
      await flushPromises()
      expect(startBus).toHaveBeenCalledWith({ id: 'main', profile: active.buses[0]?.profile, inputs: [{ source_id: 'alsa:rockchipes8388', volume: 100 }] })
      expect(wrapper.get('.page-actions button:first-child').text()).toContain('停止')
    } finally {
      wrapper.unmount()
      vi.restoreAllMocks()
      vi.unstubAllGlobals()
    }
  })
})
