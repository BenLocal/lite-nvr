import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import DetectionOverlay from './DetectionOverlay.vue'

const api = vi.hoisted(() => ({
  getDetectLatest: vi.fn(),
  listDetectModels: vi.fn(),
  startDetect: vi.fn(),
  stopDetect: vi.fn(),
}))

vi.mock('../api/detect', () => api)

describe('DetectionOverlay tap ownership', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    api.getDetectLatest.mockResolvedValue(null)
    api.listDetectModels.mockResolvedValue(['person'])
    api.stopDetect.mockResolvedValue('stopped')
  })

  function mountOverlay(persistentEnabled: boolean) {
    return mount(DetectionOverlay, {
      props: { deviceId: 'cam1', persistentEnabled },
      global: {
        stubs: {
          Button: {
            emits: ['click'],
            template: '<button type="button" @click="$emit(\'click\')"><slot /></button>',
          },
          Checkbox: { template: '<input type="checkbox" />' },
        },
      },
    })
  }

  it('views persisted detection without starting or stopping its tap', async () => {
    const wrapper = mountOverlay(true)

    await flushPromises()
    expect(api.startDetect).not.toHaveBeenCalled()
    expect(api.getDetectLatest).toHaveBeenCalledWith('cam1')

    wrapper.unmount()
    expect(api.stopDetect).not.toHaveBeenCalled()
  })

  it('does not own an already-running tap', async () => {
    api.startDetect.mockResolvedValue({ status: 'already running' })
    const wrapper = mountOverlay(false)

    await wrapper.get('.detect-toggle').trigger('click')
    await flushPromises()
    wrapper.unmount()

    expect(api.startDetect).toHaveBeenCalledWith('cam1')
    expect(api.stopDetect).not.toHaveBeenCalled()
  })

  it('stops a tap that this overlay started', async () => {
    api.startDetect.mockResolvedValue({ status: 'started', lease: '41' })
    const wrapper = mountOverlay(false)

    await wrapper.get('.detect-toggle').trigger('click')
    await flushPromises()
    await wrapper.get('.detect-toggle').trigger('click')
    await flushPromises()

    expect(api.stopDetect).toHaveBeenCalledWith('cam1', '41')
    wrapper.unmount()
    expect(api.stopDetect).toHaveBeenCalledTimes(1)
  })

  it('uses its lease so a stale overlay cannot stop a replacement tap', async () => {
    api.startDetect.mockResolvedValue({ status: 'started', lease: 'old-lease' })
    const wrapper = mountOverlay(false)

    await wrapper.get('.detect-toggle').trigger('click')
    await flushPromises()
    wrapper.unmount()

    expect(api.stopDetect).toHaveBeenCalledWith('cam1', 'old-lease')
  })
})
