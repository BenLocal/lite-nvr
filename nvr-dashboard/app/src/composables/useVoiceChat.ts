import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { createChatSocket } from '../api/chat'
import { ChatAudio } from '../utils/chatAudio'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  text: string
  state: 'pending' | 'done' | 'interrupted'
}

export function useVoiceChat() {
  const socket = createChatSocket()
  const connection = ref<'connecting' | 'connected' | 'disconnected'>('connecting')
  const phase = ref<'idle' | 'preparing' | 'listening' | 'recognizing' | 'replying'>('idle')
  const messages = ref<ChatMessage[]>([])
  const transcript = ref('')
  const error = ref('')
  const speaking = ref(false)
  const audio = new ChatAudio(value => { speaking.value = value })
  let nextTurn = 0
  let activeTurn: number | undefined
  let lastAudioSeq = -1
  let disposed = false

  const connected = computed(() => connection.value === 'connected')
  const recording = computed(() => phase.value === 'listening' || phase.value === 'preparing')
  const canSend = computed(() => connected.value && !recording.value && phase.value !== 'recognizing')
  const busy = computed(() => phase.value === 'replying' || speaking.value)
  const status = computed(() => {
    if (!connected.value) return connection.value === 'connecting' ? '正在连接…' : '连接已断开'
    if (phase.value === 'preparing') return '正在打开麦克风…'
    if (phase.value === 'listening') return '正在听，松开发送'
    if (phase.value === 'recognizing') return '正在识别语音…'
    if (speaking.value) return '机器人正在说话'
    if (phase.value === 'replying') return '机器人正在思考…'
    return '可以开始对话'
  })

  function report(cause: unknown): void {
    if (disposed) return
    error.value = cause instanceof Error ? cause.message : String(cause)
  }

  function currentReply(): ChatMessage | undefined {
    return messages.value.find(message => message.id === `assistant-${activeTurn}`)
  }

  function clearReply(): void {
    const reply = currentReply()
    if (reply?.state === 'pending') reply.state = 'interrupted'
    activeTurn = undefined
    audio.stopPlayback()
    if (phase.value === 'replying') phase.value = 'idle'
  }

  function stopReply(): void {
    if (socket.connected && activeTurn !== undefined && phase.value === 'replying') {
      socket.emit('cancel')
    }
    clearReply()
  }

  function addTurn(question: string): void {
    activeTurn = ++nextTurn
    lastAudioSeq = -1
    messages.value.push(
      { id: `user-${activeTurn}`, role: 'user', text: question, state: 'done' },
      { id: `assistant-${activeTurn}`, role: 'assistant', text: '', state: 'pending' },
    )
    phase.value = 'replying'
  }

  async function sendText(question: string): Promise<boolean> {
    const text = question.trim()
    if (!text || !canSend.value || text.length > 1000) return false
    error.value = ''
    try {
      await audio.unlock()
      if (!canSend.value || disposed) return false
      // `text` cancels the previous turn on the server itself.
      clearReply()
      addTurn(text)
      socket.emit('text', text)
      return true
    } catch (cause) {
      report(cause)
      return false
    }
  }

  async function startRecording(): Promise<void> {
    if (!canSend.value) return
    stopReply()
    error.value = ''
    transcript.value = ''
    phase.value = 'preparing'
    try {
      const started = await audio.startRecording(pcm => {
        if (connected.value && (phase.value === 'listening' || phase.value === 'recognizing')) {
          socket.emit('audio', pcm)
        }
      })
      if (!started || disposed || phase.value !== 'preparing') return
      phase.value = 'listening'
      socket.emit('start')
    } catch (cause) {
      if (phase.value !== 'preparing') return
      phase.value = 'idle'
      report(cause instanceof DOMException && cause.name === 'NotAllowedError'
        ? new Error('麦克风权限被拒绝，请在浏览器的网站设置中允许使用麦克风')
        : cause)
    }
  }

  async function stopRecording(): Promise<void> {
    if (phase.value === 'preparing') {
      phase.value = 'idle'
      await audio.stopRecording(false)
      return
    }
    if (phase.value !== 'listening') return
    phase.value = 'recognizing'
    try {
      await audio.stopRecording(true)
      if (connected.value && phase.value === 'recognizing') socket.emit('stop')
    } catch (cause) {
      phase.value = 'idle'
      socket.emit('cancel')
      report(cause)
    }
  }

  function reset(): void {
    if (!canSend.value) return
    clearReply()
    socket.emit('reset')
    messages.value = []
    transcript.value = ''
    error.value = ''
    // `reset` clears history, but server-side turn ids keep increasing.
  }

  function connect(): void {
    error.value = ''
    connection.value = 'connecting'
    socket.connect()
  }

  socket.on('connect', () => {
    connection.value = 'connected'
    phase.value = 'idle'
    nextTurn = 0
    activeTurn = undefined
    messages.value = []
    transcript.value = ''
    error.value = ''
  })
  socket.on('connect_error', cause => {
    connection.value = 'disconnected'
    error.value = cause.message === 'unauthorized'
      ? '登录已过期，请重新登录后再连接'
      : '无法连接问答机器人，请确认主机已启用语音对话服务后重试'
  })
  socket.on('disconnect', () => {
    clearReply()
    connection.value = 'disconnected'
    phase.value = 'idle'
    transcript.value = ''
    void audio.stopRecording(false).catch(report)
    error.value = '连接已断开，重新连接将开始新的对话'
  })
  socket.on('asr_partial', data => {
    if ((recording.value || phase.value === 'recognizing') && typeof data?.text === 'string') {
      transcript.value = data.text
    }
  })
  socket.on('asr_final', data => {
    if (phase.value !== 'recognizing' || typeof data?.text !== 'string') return
    transcript.value = ''
    if (data.text.trim()) addTurn(data.text)
    else {
      phase.value = 'idle'
      error.value = '没有听清，请按住按钮重新说一次'
    }
  })
  socket.on('reply_delta', data => {
    if (activeTurn === undefined || data?.turn !== activeTurn || typeof data.text !== 'string') return
    const reply = currentReply()
    if (reply) reply.text += data.text
  })
  socket.on('reply_audio', data => {
    if (activeTurn === undefined || data?.turn !== activeTurn || !Number.isInteger(data.seq) || data.seq <= lastAudioSeq) return
    lastAudioSeq = data.seq
    try { audio.play(data.pcm, data.sample_rate) } catch (cause) { report(cause) }
  })
  socket.on('reply_done', data => {
    if (activeTurn === undefined || data?.turn !== activeTurn) return
    const reply = currentReply()
    if (reply?.state === 'pending') reply.state = data.interrupted ? 'interrupted' : 'done'
    phase.value = 'idle'
    if (data.interrupted) audio.stopPlayback()
  })
  socket.on('error', data => {
    if (typeof data?.message !== 'string' || (data.turn !== undefined && data.turn !== activeTurn)) return
    error.value = data.message
    if (data.turn === undefined) {
      phase.value = 'idle'
      void audio.stopRecording(false).catch(report)
    }
  })

  onMounted(connect)
  onBeforeUnmount(() => {
    disposed = true
    socket.removeAllListeners()
    socket.disconnect()
    void audio.dispose().catch(() => { /* Resources are already detached on exit. */ })
  })

  return {
    connection, connected, phase, messages, transcript, error, recording, canSend, busy, status,
    sendText, startRecording, stopRecording, stopReply, reset, connect,
  }
}
