interface Capture {
  stream: MediaStream
  source: MediaStreamAudioSourceNode
  node: AudioWorkletNode
  muted: GainNode
}

/** Browser microphone and queued sentence playback for the /chat wire format. */
export class ChatAudio {
  private context?: AudioContext
  private capture?: Capture
  private captureVersion = 0
  private disposed = false
  private workletReady?: Promise<void>
  private sources = new Set<AudioBufferSourceNode>()
  private playAt = 0

  constructor(private readonly onPlaying: (playing: boolean) => void) {}

  async unlock(): Promise<void> {
    if (this.disposed) throw new Error('对话已关闭')
    if (!window.AudioContext) throw new Error('当前浏览器不支持语音播放，请换用新版浏览器')
    this.context ??= new AudioContext({ sampleRate: 16000 })
    if (this.context.state === 'suspended') await this.context.resume()
  }

  async startRecording(onAudio: (pcm: ArrayBuffer) => void): Promise<boolean> {
    const version = ++this.captureVersion
    if (!navigator.mediaDevices?.getUserMedia || !window.isSecureContext) {
      throw new Error('麦克风需要 HTTPS 或 localhost，请通过安全地址打开页面')
    }
    await this.unlock()
    const context = this.context
    if (!context?.audioWorklet) throw new Error('当前浏览器不支持语音采集，请换用新版浏览器')
    this.workletReady ??= context.audioWorklet.addModule(`${import.meta.env.BASE_URL}audio/pcm-capture.js`)
    await this.workletReady
    if (version !== this.captureVersion || this.disposed) return false
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    })
    if (version !== this.captureVersion || this.disposed) {
      stream.getTracks().forEach(track => track.stop())
      return false
    }
    try {
      const source = context.createMediaStreamSource(stream)
      const node = new AudioWorkletNode(context, 'chat-pcm-capture', {
        channelCount: 1,
        channelCountMode: 'explicit',
      })
      const muted = context.createGain()
      muted.gain.value = 0
      node.port.onmessage = ({ data }: MessageEvent<unknown>) => {
        if (data instanceof ArrayBuffer) onAudio(data)
      }
      source.connect(node).connect(muted).connect(context.destination)
      this.capture = { stream, source, node, muted }
      return true
    } catch (error) {
      stream.getTracks().forEach(track => track.stop())
      throw error
    }
  }

  async stopRecording(flush: boolean): Promise<void> {
    ++this.captureVersion
    const capture = this.capture
    this.capture = undefined
    if (!capture) return
    capture.source.disconnect()
    capture.stream.getTracks().forEach(track => track.stop())
    try {
      if (flush) {
        // Deliver the last partial frame before the caller emits `stop`.
        await new Promise<void>((resolve, reject) => {
          const timeout = setTimeout(() => reject(new Error('语音采集结束超时，请重新录音')), 1000)
          const receive = capture.node.port.onmessage
          capture.node.port.onmessage = event => {
            if (event.data === 'flushed') {
              clearTimeout(timeout)
              resolve()
            } else {
              receive?.call(capture.node.port, event)
            }
          }
          capture.node.port.postMessage('flush')
        })
      }
    } finally {
      capture.node.port.onmessage = null
      capture.node.port.close()
      capture.node.disconnect()
      capture.muted.disconnect()
    }
  }

  play(pcm: ArrayBuffer, sampleRate: number): void {
    const context = this.context
    if (!context || this.disposed) return
    if (!(pcm instanceof ArrayBuffer) || pcm.byteLength % 2 !== 0 ||
        !Number.isInteger(sampleRate) || sampleRate < 8000 || sampleRate > 192000) {
      throw new Error('收到的语音格式无效')
    }
    if (!pcm.byteLength) return
    const buffer = context.createBuffer(1, pcm.byteLength / 2, sampleRate)
    const channel = buffer.getChannelData(0)
    const view = new DataView(pcm)
    for (let i = 0; i < channel.length; i++) channel[i] = view.getInt16(i * 2, true) / 32768
    const source = context.createBufferSource()
    source.buffer = buffer
    source.connect(context.destination)
    source.onended = () => {
      this.sources.delete(source)
      source.disconnect()
      this.onPlaying(this.sources.size > 0)
    }
    this.sources.add(source)
    this.playAt = Math.max(context.currentTime, this.playAt)
    source.start(this.playAt)
    this.playAt += buffer.duration
    this.onPlaying(true)
  }

  stopPlayback(): void {
    for (const source of this.sources) {
      source.onended = null
      source.stop()
      source.disconnect()
    }
    this.sources.clear()
    this.playAt = 0
    this.onPlaying(false)
  }

  async dispose(): Promise<void> {
    this.disposed = true
    this.stopPlayback()
    await this.stopRecording(false)
    await this.context?.close()
  }
}
