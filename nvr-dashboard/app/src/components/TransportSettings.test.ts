import { mount, flushPromises } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import PrimeVue from 'primevue/config'
import { listDevices } from '../api/device'
import TransportSettings from './TransportSettings.vue'
import { getTransportCapabilities, listTransportTargets, saveTransportTarget } from '../api/transport'

vi.mock('../api/transport', () => ({
  getTransportCapabilities: vi.fn(), listTransportTargets: vi.fn(), saveTransportTarget: vi.fn(),
  removeTransportTarget: vi.fn(), testTransportTarget: vi.fn(),
}))
vi.mock('../api/device', () => ({ listDevices: vi.fn().mockResolvedValue([{ id: 'cam', name: 'Camera' }]) }))
vi.mock('../utils/toast', () => ({ useAppToast: () => ({ success: vi.fn(), warn: vi.fn(), errorFrom: vi.fn() }) }))
vi.mock('primevue/useconfirm', () => ({ useConfirm: () => ({ require: vi.fn() }) }))

const target = {
  id: 'archive', name: 'Archive', kind: 'ftp', enabled: false,
  config: { host: 'server', port: 21, username: '', password: '', base_path: '', stream_ids: ['cam'] },
  remark: '', done: 0, failed: 0,
}

async function mountSettings() {
  const wrapper = mount(TransportSettings, {
    global: { plugins: [PrimeVue], stubs: { teleport: true, transition: false } },
  })
  await flushPromises()
  return wrapper
}

describe('transport configuration', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(listDevices).mockResolvedValue([])
    vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })))
    vi.mocked(getTransportCapabilities).mockResolvedValue({ smb_enabled: false })
    vi.mocked(listTransportTargets).mockResolvedValue([])
    vi.mocked(saveTransportTarget).mockResolvedValue()
  })

  it('saves a new FTP destination disabled with all-device routing', async () => {
    const wrapper = await mountSettings()
    await wrapper.findAll('button').find((button) => button.text() === '新增目标')?.trigger('click')
    await flushPromises()
    await wrapper.get('#transport-name').setValue('Archive')
    await wrapper.get('#transport-host').setValue('server')
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(saveTransportTarget).toHaveBeenCalledWith(null, expect.objectContaining({
      kind: 'ftp', enabled: false, config: expect.objectContaining({ host: 'server', port: 21, stream_ids: null }),
    }))
    wrapper.unmount()
  })

  it('preserves selected devices and sends a blank password when editing', async () => {
    vi.mocked(listTransportTargets).mockResolvedValue([{
      ...target, kind: 'ftp',
    }])
    const wrapper = await mountSettings()
    await wrapper.findAll('button').find((button) => button.text() === '编辑')?.trigger('click')
    await flushPromises()
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(saveTransportTarget).toHaveBeenCalledWith('archive', expect.objectContaining({
      config: expect.objectContaining({ password: '', stream_ids: ['cam'] }),
    }))
    wrapper.unmount()
  })

  it('can disable an existing SMB destination after SMB support is removed', async () => {
    vi.mocked(listTransportTargets).mockResolvedValue([{
      ...target, kind: 'smb', enabled: true, config: { ...target.config, share: 'records' },
    }])
    const wrapper = await mountSettings()
    await wrapper.findAll('button').find((button) => button.text() === '编辑')?.trigger('click')
    await flushPromises()
    expect(wrapper.get('#transport-enabled').attributes('disabled')).toBeDefined()
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(saveTransportTarget).toHaveBeenCalledWith('archive', expect.objectContaining({ kind: 'smb', enabled: false }))
    wrapper.unmount()
  })

  it('shows a load failure and prevents creating a target while unavailable', async () => {
    vi.mocked(getTransportCapabilities).mockRejectedValue(new Error('offline'))
    const wrapper = await mountSettings()
    expect(wrapper.text()).toContain('offline')
    expect(wrapper.findAll('button').find((button) => button.text() === '新增目标')?.attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })
})
