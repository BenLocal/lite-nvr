import { defineComponent } from 'vue'
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useVoiceChat } from './useVoiceChat'

const fake = vi.hoisted(() => ({
  turn: 0,
  handlers: new Map<string, (data?: unknown) => void>(),
  emit: vi.fn(),
  connect: vi.fn(),
  disconnect: vi.fn(),
  removeAllListeners: vi.fn(),
  unlock: vi.fn(),
  startRecording: vi.fn(),
  stopRecording: vi.fn(),
  play: vi.fn(),
  stopPlayback: vi.fn(),
  dispose: vi.fn(),
}))

vi.mock('../api/chat', () => ({
  createChatSocket: () => ({
    connected: true,
    on: (name: string, handler: (data?: unknown) => void) => fake.handlers.set(name, handler),
    emit: fake.emit,
    connect: fake.connect,
    disconnect: fake.disconnect,
    removeAllListeners: fake.removeAllListeners,
  }),
}))
vi.mock('../utils/chatAudio', () => ({
  ChatAudio: class {
    unlock = fake.unlock
    startRecording = fake.startRecording
    stopRecording = fake.stopRecording
    play = fake.play
    stopPlayback = fake.stopPlayback
    dispose = fake.dispose
  },
}))

function receive(name: string, data?: unknown): void {
  fake.handlers.get(name)?.(data)
}

type Ack = (data: { turn: number | null }) => void

/** Acks `text` like the server: each question starts the next turn. */
function serverEmit(event: string, _data?: unknown, ack?: Ack): void {
  if (event === 'text') ack?.({ turn: ++fake.turn })
}

let chat: ReturnType<typeof useVoiceChat>
let unmount: () => void

beforeEach(() => {
  vi.clearAllMocks()
  fake.emit.mockReset()
  fake.emit.mockImplementation(serverEmit)
  fake.turn = 0
  fake.handlers.clear()
  fake.unlock.mockResolvedValue(undefined)
  fake.startRecording.mockResolvedValue(true)
  fake.stopRecording.mockResolvedValue(undefined)
  fake.dispose.mockResolvedValue(undefined)
  const wrapper = mount(defineComponent({
    setup() { chat = useVoiceChat(); return () => null },
  }))
  unmount = () => wrapper.unmount()
  receive('connect')
})

afterEach(() => unmount())

describe('voice chat session', () => {
  it('does not cancel the first typed question when server callbacks run out of order', async () => {
    const queued: string[] = []
    fake.emit.mockImplementation((event: string, data?: unknown, ack?: Ack) => {
      queued.push(event)
      serverEmit(event, data, ack)
    })
    await chat.sendText('你好')
    // Old async Socket.IO handlers can enqueue `text` before the preceding
    // `cancel`. Reproduce that order at the real composable's event boundary.
    let running = false
    for (const event of queued.reverse()) {
      if (event === 'text') running = true
      if (event === 'cancel' && running) {
        receive('reply_done', { turn: 1, interrupted: true })
        running = false
      }
    }
    if (running) {
      receive('reply_delta', { turn: 1, text: '你好！' })
      receive('reply_done', { turn: 1, interrupted: false })
    }
    expect(chat.messages.value[1]?.text).toBe('你好！')
    expect(chat.messages.value[1]?.state).toBe('done')
  })

  it('uses the next question itself to interrupt a reply without sending an extra cancel', async () => {
    await chat.sendText('第一轮')
    fake.emit.mockClear()
    await chat.sendText('第二轮')
    expect(fake.emit.mock.calls).toEqual([['text', '第二轮', expect.any(Function)]])
    expect(chat.messages.value[1]?.state).toBe('interrupted')
    receive('reply_delta', { turn: 2, text: '第二轮的回答' })
    expect(chat.messages.value[3]?.text).toBe('第二轮的回答')
  })

  it('starts voice input without a separate cancellation event', async () => {
    await chat.startRecording()
    expect(fake.emit.mock.calls).toEqual([['start']])
  })

  it('renders a streamed reply and ignores cancelled turns and duplicate audio', async () => {
    await chat.sendText('你好')
    receive('reply_delta', { turn: 1, text: '你好，' })
    receive('reply_delta', { turn: 1, text: '有什么问题？' })
    expect(chat.messages.value[1]?.text).toBe('你好，有什么问题？')
    await chat.sendText('第二个问题')
    receive('reply_delta', { turn: 1, text: '旧回复' })
    receive('reply_done', { turn: 1, interrupted: true })
    receive('reply_audio', { turn: 1, seq: 0, pcm: new ArrayBuffer(2), sample_rate: 16000 })
    expect(chat.phase.value).toBe('replying')
    expect(chat.messages.value[1]?.state).toBe('interrupted')
    expect(chat.messages.value[1]?.text).not.toContain('旧回复')
    const frame = { turn: 2, seq: 0, pcm: new ArrayBuffer(2), sample_rate: 22050 }
    receive('reply_audio', frame)
    receive('reply_audio', frame)
    expect(fake.play).toHaveBeenCalledTimes(1)
    receive('reply_done', { turn: 2, interrupted: false })
    expect(chat.phase.value).toBe('idle')
    expect(chat.messages.value[3]?.state).toBe('done')
  })

  it('sends the last microphone frame before stop, then displays the recognized question', async () => {
    let emitAudio: ((pcm: ArrayBuffer) => void) | undefined
    fake.startRecording.mockImplementation(async (onAudio: (pcm: ArrayBuffer) => void) => {
      emitAudio = onAudio
      return true
    })
    fake.stopRecording.mockImplementation(async () => emitAudio?.(new ArrayBuffer(6)))
    await chat.startRecording()
    expect(chat.recording.value).toBe(true)
    receive('asr_partial', { text: '北京' })
    expect(chat.transcript.value).toBe('北京')
    await chat.stopRecording()
    expect(fake.emit.mock.calls.slice(-2)).toEqual([['audio', new ArrayBuffer(6)], ['stop']])
    receive('asr_final', { text: '北京有哪些景点？', turn: 1 })
    expect(chat.messages.value[0]?.text).toBe('北京有哪些景点？')
    expect(chat.phase.value).toBe('replying')
    receive('reply_delta', { turn: 1, text: '故宫' })
    expect(chat.messages.value[1]?.text).toBe('故宫')
  })

  it('does not start a turn when recognition is empty', async () => {
    await chat.startRecording()
    await chat.stopRecording()
    receive('asr_final', { text: '', turn: null })
    expect(chat.messages.value).toEqual([])
    expect(chat.phase.value).toBe('idle')
    expect(chat.error.value).toContain('没有听清')
  })

  it('can release while waiting for microphone permission', async () => {
    let resolvePermission: ((started: boolean) => void) | undefined
    fake.startRecording.mockImplementation(() => new Promise<boolean>(resolve => { resolvePermission = resolve }))
    const opening = chat.startRecording()
    await chat.stopRecording()
    resolvePermission?.(false)
    await opening
    expect(chat.phase.value).toBe('idle')
    expect(fake.emit).not.toHaveBeenCalledWith('start')
    expect(fake.stopRecording).toHaveBeenCalledWith(false)
  })

  it('preserves server turn numbering across reset and rejects stale replies', async () => {
    await chat.sendText('第一轮')
    chat.reset()
    receive('reply_delta', { turn: 1, text: '延迟到达' })
    expect(chat.messages.value).toEqual([])
    await chat.sendText('新的对话')
    receive('reply_delta', { turn: 2, text: '新的回答' })
    expect(chat.messages.value[1]?.text).toBe('新的回答')
  })

  it('follows the server turn id after a recognized question the client ignored', async () => {
    // The server started turn 1 from speech while the client was not recognizing.
    receive('asr_final', { text: '没等到的问题', turn: ++fake.turn })
    expect(chat.messages.value).toEqual([])
    await chat.sendText('下一个问题')
    receive('reply_delta', { turn: 1, text: '旧回复' })
    receive('reply_delta', { turn: 2, text: '新回复' })
    expect(chat.messages.value[1]?.text).toBe('新回复')
  })

  it('ignores the ack of a question superseded before the server answered it', async () => {
    const acks: Ack[] = []
    fake.emit.mockImplementation((event: string, _data?: unknown, ack?: Ack) => {
      if (event === 'text' && ack) acks.push(ack)
    })
    await chat.sendText('第一轮')
    await chat.sendText('第二轮')
    acks[0]?.({ turn: 1 })
    receive('reply_delta', { turn: 1, text: '旧回复' })
    receive('reply_done', { turn: 1, interrupted: true })
    expect(chat.messages.value[3]?.state).toBe('pending')
    acks[1]?.({ turn: 2 })
    receive('reply_delta', { turn: 2, text: '新回复' })
    expect(chat.messages.value[3]?.text).toBe('新回复')
    expect(chat.messages.value[1]?.text).toBe('')
  })

  it('stops microphone and playback on disconnect and starts a fresh session after reconnect', async () => {
    await chat.sendText('你好')
    receive('disconnect')
    await flushPromises()
    expect(chat.canSend.value).toBe(false)
    expect(fake.stopPlayback).toHaveBeenCalled()
    expect(fake.stopRecording).toHaveBeenCalledWith(false)
    chat.connect()
    fake.turn = 0
    receive('connect')
    expect(chat.messages.value).toEqual([])
    await chat.sendText('重连后的第一轮')
    receive('reply_delta', { turn: 1, text: '收到' })
    expect(chat.messages.value[1]?.text).toBe('收到')
  })

  it('reports microphone denial and releases all resources when leaving', async () => {
    fake.startRecording.mockRejectedValue(new DOMException('denied', 'NotAllowedError'))
    await chat.startRecording()
    expect(chat.phase.value).toBe('idle')
    expect(chat.error.value).toContain('权限被拒绝')
    unmount()
    expect(fake.removeAllListeners).toHaveBeenCalled()
    expect(fake.disconnect).toHaveBeenCalled()
    expect(fake.dispose).toHaveBeenCalled()
  })
})
