import { mount, flushPromises } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import PrimeVue from 'primevue/config'
import PlaybackView from './PlaybackView.vue'
import {
  listPlayback, listDevicePlaybackSegments, listTodayDevicePlaybackSegments,
  type PlaybackSegmentItem,
} from '../api/playback'

const hlsMock = vi.hoisted(() => ({
  isSupported: vi.fn(), loadSource: vi.fn(), attachMedia: vi.fn(),
  on: vi.fn(), destroy: vi.fn(),
}))
vi.mock('hls.js', () => ({
  default: class {
    static isSupported = hlsMock.isSupported
    static Events = { ERROR: 'hlsError' }
    loadSource = hlsMock.loadSource
    attachMedia = hlsMock.attachMedia
    on = hlsMock.on
    destroy = hlsMock.destroy
  },
}))

vi.mock('../api/playback', async (importOriginal) => ({
  ...await importOriginal<typeof import('../api/playback')>(),
  listPlayback: vi.fn(),
  listDevicePlaybackSegments: vi.fn(),
  listTodayDevicePlaybackSegments: vi.fn(),
}))
vi.mock('../auth/token', () => ({ getAuthToken: () => null }))
vi.mock('../utils/toast', () => ({ useAppToast: () => ({ info: vi.fn(), error: vi.fn(), errorFrom: vi.fn() }) }))
vi.mock('primevue/useconfirm', () => ({ useConfirm: () => ({ require: vi.fn() }) }))

function segment(id: string, offset: number): PlaybackSegmentItem {
  const now = new Date()
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 12).getTime() / 1000
  return {
    id, start_time: start + offset, duration: 60, file_size: 1024,
    file_name: `${id}.ts`, file_path: `/records/${id}.ts`,
    video_codec: 'h264', video_width: 1920, video_height: 1080, video_fps: 60, video_bit_rate: 0,
    audio_codec: '', audio_sample_rate: 0, audio_channels: 0, audio_bit_rate: 0,
    create_time: '', update_time: '',
  }
}

const first = segment('first', 0)
const second = segment('second', 60)
const last = segment('last', 120)

async function mountPlayback() {
  const wrapper = mount(PlaybackView, {
    global: { plugins: [PrimeVue], stubs: { teleport: true, transition: false } },
  })
  await flushPromises()
  return wrapper
}

describe('playback segment continuation', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    hlsMock.isSupported.mockReturnValue(false)
    vi.spyOn(HTMLMediaElement.prototype, 'canPlayType').mockReturnValue('probably')
    vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => {})
    vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {})
    vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
    vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })))
    vi.mocked(listPlayback).mockResolvedValue({
      items: [{ device_id: 'camera', device_name: 'Camera', input_type: 'v4l2', segment_count: 3 }],
      page: 1, page_size: 12, total: 1,
    })
    // The next chronological segment is outside the visible list page.
    vi.mocked(listDevicePlaybackSegments).mockResolvedValue({ items: [first], page: 2, page_size: 1, total: 3 })
    vi.mocked(listTodayDevicePlaybackSegments).mockResolvedValue([last, first, second])
  })

  it('uses hls.js when MSE is supported even if native HLS is advertised', async () => {
    hlsMock.isSupported.mockReturnValue(true)
    const wrapper = await mountPlayback()
    await wrapper.findAll('button').find((button) => button.text() === '打开回放')?.trigger('click')
    await flushPromises()
    expect(hlsMock.loadSource).toHaveBeenCalledWith('/api/playback/playlist/camera')
    expect(hlsMock.attachMedia).toHaveBeenCalledWith(wrapper.get('video').element)
    wrapper.unmount()
    expect(hlsMock.destroy).toHaveBeenCalled()
  })

  it('continues in chronological order after starting from a segment card', async () => {
    const wrapper = await mountPlayback()
    await wrapper.get('button[aria-label="播放"]').trigger('click')
    await flushPromises()
    const video = wrapper.get('video')
    expect(video.attributes('src')).toBe('/api/playback/segment-playlist/first')
    await video.trigger('ended')
    await flushPromises()
    expect(video.attributes('src')).toBe('/api/playback/segment-playlist/second')
    await video.trigger('loadedmetadata')
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalled()
    await video.trigger('ended')
    await flushPromises()
    expect(video.attributes('src')).toBe('/api/playback/segment-playlist/last')
    await video.trigger('ended')
    await flushPromises()
    expect(video.attributes('src')).toBe('/api/playback/segment-playlist/last')
    wrapper.unmount()
  })

  it('shows the current segment index and total during daily playback', async () => {
    vi.mocked(listTodayDevicePlaybackSegments).mockResolvedValue([first, second, last])
    const wrapper = await mountPlayback()
    await wrapper.findAll('button').find((button) => button.text() === '打开回放')?.trigger('click')
    await flushPromises()
    const video = wrapper.get('video')
    await video.trigger('loadedmetadata')
    expect(wrapper.get('.timeline-stats').text()).toContain('1 / 3')
    video.element.currentTime = 60
    await video.trigger('timeupdate')
    expect(wrapper.get('.timeline-stats').text()).toContain('2 / 3')
    video.element.currentTime = 123
    await video.trigger('timeupdate')
    expect(wrapper.get('.timeline-stats').text()).toContain('3 / 3')
    expect(wrapper.get('.timeline-stats').text()).toContain('3s / 1m 0s')
    wrapper.unmount()
  })
})
