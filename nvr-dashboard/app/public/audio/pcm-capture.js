// Socket.IO /chat accepts 100 ms chunks of mono, 16 kHz s16le PCM.
class PcmCapture extends AudioWorkletProcessor {
  constructor() {
    super()
    this.phase = 0
    this.sum = 0
    this.count = 0
    this.samples = []
    this.flushed = false
    this.port.onmessage = ({ data }) => {
      if (data === 'flush') {
        this.send()
        this.flushed = true
        this.port.postMessage('flushed')
      }
    }
  }

  send() {
    if (!this.samples.length) return
    const pcm = new ArrayBuffer(this.samples.length * 2)
    const view = new DataView(pcm)
    this.samples.forEach((sample, i) => view.setInt16(i * 2, sample, true))
    this.samples = []
    this.port.postMessage(pcm, [pcm])
  }

  process(inputs) {
    const input = inputs[0]?.[0]
    if (!input || this.flushed) return true
    // Usually sampleRate is already 16 kHz. Preserve phase across callbacks
    // when the browser uses its hardware rate (for example 44.1 or 48 kHz).
    for (const sample of input) {
      this.sum += sample
      this.count++
      this.phase += 16000
      if (this.phase >= sampleRate) {
        const value = Math.max(-1, Math.min(1, this.sum / this.count))
        this.samples.push(Math.round(value * (value < 0 ? 32768 : 32767)))
        this.phase -= sampleRate
        this.sum = 0
        this.count = 0
        if (this.samples.length === 1600) this.send()
      }
    }
    return true
  }
}

registerProcessor('chat-pcm-capture', PcmCapture)
