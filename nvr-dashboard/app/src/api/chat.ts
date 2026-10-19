import { io, type Socket } from 'socket.io-client'
import { getAuthToken } from '../auth/token'

export interface ChatServerEvents {
  asr_partial: (data: { text: string }) => void
  /** `turn` answers the recognized question; null when nothing was heard. */
  asr_final: (data: { text: string; turn: number | null }) => void
  reply_delta: (data: { turn: number; text: string }) => void
  reply_audio: (data: { turn: number; seq: number; sample_rate: number; pcm: ArrayBuffer }) => void
  reply_done: (data: { turn: number; interrupted: boolean }) => void
  error: (data: { turn?: number; message: string }) => void
}

interface ChatClientEvents {
  start: () => void
  stop: () => void
  audio: (pcm: ArrayBuffer) => void
  /** The ack carries the turn that answers the question. */
  text: (question: string, ack: (data: { turn: number | null }) => void) => void
  cancel: () => void
  reset: () => void
}

export type ChatSocket = Socket<ChatServerEvents, ChatClientEvents>

export function createChatSocket(): ChatSocket {
  // A connection owns its history. Reconnect explicitly so a lost session is
  // never silently presented as a continuation of the previous conversation.
  return io('/chat', {
    auth: { token: getAuthToken() },
    transports: ['websocket'],
    autoConnect: false,
    reconnection: false,
    timeout: 10000,
  })
}
