import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { describe, expect, it } from 'vitest'

interface Processor {
  port: { onmessage: (event: { data: string }) => void }
  process: (inputs: Float32Array[][]) => boolean
}

function recorder(sampleRate: number): { processor: Processor; messages: unknown[] } {
  const messages: unknown[] = []
  let makeProcessor: (() => Processor) | undefined
  runInNewContext(readFileSync('public/audio/pcm-capture.js', 'utf8'), {
    sampleRate,
    ArrayBuffer,
    DataView,
    AudioWorkletProcessor: class {
      port = { postMessage: (data: unknown) => messages.push(data) }
    },
    registerProcessor: (_name: string, constructor: new () => Processor) => {
      makeProcessor = () => new constructor()
    },
  })
  if (!makeProcessor) throw new Error('Worklet was not registered')
  return { processor: makeProcessor(), messages }
}

describe('microphone PCM worklet', () => {
  for (const rate of [16000, 44100, 48000]) {
    it(`resamples ${rate} Hz into 16 kHz little-endian frames without drift`, () => {
      const { processor, messages } = recorder(rate)
      // Feed render-size blocks to catch rounding errors at callback boundaries.
      for (let sent = 0; sent < rate; sent += 128) {
        processor.process([[new Float32Array(Math.min(128, rate - sent)).fill(0.5)]])
      }
      processor.port.onmessage({ data: 'flush' })
      const frames = messages.filter((value): value is ArrayBuffer => value instanceof ArrayBuffer)
      expect(frames).toHaveLength(10)
      expect(frames.every(frame => frame.byteLength === 3200)).toBe(true)
      expect(frames.reduce((sum, frame) => sum + frame.byteLength, 0)).toBe(32000)
      expect(new DataView(frames[0] ?? new ArrayBuffer(0)).getInt16(0, true)).toBe(16384)
      expect(messages[messages.length - 1]).toBe('flushed')
    })
  }

  it('flushes a partial final frame and ignores input after flush', () => {
    const { processor, messages } = recorder(16000)
    processor.process([[Float32Array.of(-1, 1, 0)]])
    processor.port.onmessage({ data: 'flush' })
    const frame = messages[0]
    expect(frame).toBeInstanceOf(ArrayBuffer)
    if (!(frame instanceof ArrayBuffer)) throw new Error('Missing PCM')
    const view = new DataView(frame)
    expect([view.getInt16(0, true), view.getInt16(2, true), view.getInt16(4, true)]).toEqual([-32768, 32767, 0])
    processor.process([[new Float32Array(1600)]])
    expect(messages).toHaveLength(2)
  })
})
