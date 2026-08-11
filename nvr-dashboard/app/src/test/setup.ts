import { vi } from 'vitest'

class TestResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

vi.stubGlobal('ResizeObserver', TestResizeObserver)

HTMLCanvasElement.prototype.getContext = vi.fn(() => null)
