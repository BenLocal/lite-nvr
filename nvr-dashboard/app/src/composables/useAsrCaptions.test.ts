import { describe, expect, it } from 'vitest'
import { finalLines } from './useAsrCaptions'

describe('finalLines', () => {
  it('uses the backend sentence split', () => {
    expect(
      finalLines({
        id: 1,
        final: true,
        text: '你好。再见！',
        sentences: [
          { text: '你好。', start: 0, duration: 1 },
          { text: '再见！', start: 1, duration: 1 },
        ],
      }),
    ).toEqual(['你好。', '再见！'])
  })

  it('falls back to the whole text without sentences', () => {
    expect(finalLines({ text: '旧格式' })).toEqual(['旧格式'])
    expect(finalLines({ sentences: [] })).toEqual([])
  })
})
