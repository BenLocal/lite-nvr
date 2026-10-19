import { computed, onBeforeUnmount, onMounted, ref, type ComputedRef, type Ref } from 'vue'
import { createChatSocket } from '../api/chat'
import { ChatAudio } from '../utils/chatAudio'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  text: string
  state: 'pending' | 'done' | 'interrupted'
}

export interface VoiceChat {
  connection: Ref<'connecting' | 'connected' | 'disconnected'>
  connected: ComputedRef<boolean>
  phase: Ref<'idle' | 'preparing' | 'listening' | 'recognizing' | 'replying'>
  messages: Ref<ChatMessage[]>
  transcript: Ref<string>
  error: Ref<string>
  recording: ComputedRef<boolean>
  canSend: ComputedRef<boolean>
  busy: ComputedRef<boolean>
  status: ComputedRef<string>
  sendText: (question: string) => Promise<boolean>
  startRecording: () => Promise<void>
  stopRecording: () => Promise<void>
  stopReply: () => void
  reset: () => void
  connect: () => void
}

export function useVoiceChat(): VoiceChat {
  const socket = createChatSocket()
  const connection = ref<'connecting' | 'connected' | 'disconnected'>('connecting')
  const phase = ref<'idle' | 'preparing' | 'listening' | 'recognizing' | 'replying'>('idle')
  const messages = ref<ChatMessage[]>([])
  const transcript = ref('')
  const error = ref('')
  const speaking = ref(false)
  const audio = new ChatAudio(value => { speaking.value = value })
  let nextMessage = 0
  // Turn ids come from the server; replies stay hidden until it names the turn.
  let activeTurn: number | undefined
  let activeReply: string | undefined
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
    console.error('voice chat:', cause)
    if (disposed) return
    error.value = cause instanceof Error ? cause.message : String(cause)
  }

  function currentReply(): ChatMessage | undefined {
    return messages.value.find(message => message.id === activeReply)
  }

  function clearReply(): void {
    const reply = currentReply()
    if (reply?.state === 'pending') reply.state = 'interrupted'
    activeTurn = undefined
    activeReply = undefined
    audio.stopPlayback()
    if (phase.value === 'replying') phase.value = 'idle'
  }

  function stopReply(): void {
    if (socket.connected && activeReply !== undefined && phase.value === 'replying') {
      socket.emit('cancel')
    }
    clearReply()
  }

  /** Returns the id of the pending reply. */
  function addTurn(question: string): string {
    const id = ++nextMessage
    activeTurn = undefined
    activeReply = `assistant-${id}`
    lastAudioSeq = -1
    messages.value.push(
      { id: `user-${id}`, role: 'user', text: question, state: 'done' },
      { id: activeReply, role: 'assistant', text: '', state: 'pending' },
    )
    phase.value = 'replying'
    return activeReply
  }

  /** Binds the pending reply `id` to the server's `turn`, unless it was superseded. */
  function startTurn(id: string, turn: unknown): void {
    if (id !== activeReply) return
    if (typeof turn === 'number') activeTurn = turn
    else clearReply()
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
      const id = addTurn(text)
      socket.emit('text', text, data => startTurn(id, data?.turn))
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
  }

  function connect(): void {
    error.value = ''
    connection.value = 'connecting'
    socket.connect()
  }

  socket.on('connect', () => {
    connection.value = 'connected'
    phase.value = 'idle'
    activeTurn = undefined
    activeReply = undefined
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
    if (data.text.trim() && typeof data.turn === 'number') startTurn(addTurn(data.text), data.turn)
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
    void audio.dispose().catch(report)
  })

  return {
    connection, connected, phase, messages, transcript, error, recording, canSend, busy, status,
    sendText, startRecording, stopRecording, stopReply, reset, connect,
  }
}
