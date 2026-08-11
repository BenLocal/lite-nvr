import { beforeEach, describe, expect, it, vi } from 'vitest'
import { startDetect, stopDetect } from './detect'

describe('detection tap leases', () => {
  const fetchMock = vi.fn<typeof fetch>()

  beforeEach(() => {
    fetchMock.mockReset()
    vi.stubGlobal('fetch', fetchMock)
    window.localStorage.clear()
  })

  it('returns the lease attached to a newly started tap', async () => {
    fetchMock.mockResolvedValue(
      new Response('started', {
        status: 200,
        headers: { 'x-detection-tap-lease': '41' },
      }),
    )

    await expect(startDetect('cam 1')).resolves.toEqual({ status: 'started', lease: '41' })
  })

  it('sends the lease when stopping an owned tap', async () => {
    fetchMock.mockResolvedValue(new Response('stopped', { status: 200 }))

    await stopDetect('cam 1', '41')

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/detect/cam%201/stop',
      expect.objectContaining({ method: 'POST', body: JSON.stringify({ lease: '41' }) }),
    )
  })
})
