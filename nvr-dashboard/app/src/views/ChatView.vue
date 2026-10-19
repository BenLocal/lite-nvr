<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Button from 'primevue/button'
import Textarea from 'primevue/textarea'
import { useVoiceChat } from '../composables/useVoiceChat'

const chat = useVoiceChat()
const { connection, connected, phase, messages, transcript, error, recording, canSend, busy, status } = chat
const question = ref('')
const conversation = ref<HTMLElement>()
const secureMicrophone = window.isSecureContext && !!navigator.mediaDevices?.getUserMedia
let held = false

async function send(): Promise<void> {
  const text = question.value
  if (await chat.sendText(text) && question.value === text) question.value = ''
}

function composerKeyDown(event: KeyboardEvent): void {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
    event.preventDefault()
    void send()
  }
}

function press(event: PointerEvent): void {
  if (event.button !== 0 || held || !canSend.value) return
  event.preventDefault()
  held = true
  if (event.currentTarget instanceof HTMLElement) event.currentTarget.setPointerCapture(event.pointerId)
  void chat.startRecording()
}

function release(): void {
  held = false
  void chat.stopRecording()
}

function keyDown(event: KeyboardEvent): void {
  if ((event.key === ' ' || event.key === 'Enter') && !event.repeat && !held) {
    event.preventDefault()
    held = true
    void chat.startRecording()
  }
}

function keyUp(event: KeyboardEvent): void {
  if (event.key === ' ' || event.key === 'Enter') {
    event.preventDefault()
    release()
  }
}

function visibilityChanged(): void {
  if (document.hidden) release()
}

watch(() => [messages.value.length, messages.value[messages.value.length - 1]?.text, transcript.value], async () => {
  await nextTick()
  conversation.value?.scrollTo({ top: conversation.value.scrollHeight, behavior: 'instant' })
})

onMounted(() => {
  window.addEventListener('blur', release)
  document.addEventListener('visibilitychange', visibilityChanged)
})
onBeforeUnmount(() => {
  window.removeEventListener('blur', release)
  document.removeEventListener('visibilitychange', visibilityChanged)
})
</script>

<template>
  <section class="content-section chat-page">
    <header class="page-header">
      <div class="header-content">
        <h1 class="page-title">问答机器人</h1>
        <p class="page-subtitle">按住说话，松开发送；也可以输入文字提问</p>
      </div>
      <div class="page-actions">
        <span class="chat-connection" :class="{ 'chat-connected': connected }" role="status">{{ connected ? '已连接' : connection === 'connecting' ? '连接中' : '未连接' }}</span>
        <Button v-if="connection === 'disconnected'" label="重新连接" icon="pi pi-refresh" severity="secondary" outlined @click="chat.connect" />
        <Button label="新建对话" icon="pi pi-plus" severity="secondary" outlined :disabled="!canSend" @click="chat.reset" />
      </div>
    </header>

    <div class="chat-panel">
      <div ref="conversation" class="chat-conversation" role="log" aria-label="对话记录" aria-live="polite" aria-relevant="additions text">
        <div v-if="!messages.length" class="chat-welcome">
          <i class="pi pi-comments" aria-hidden="true" />
          <h2>想聊些什么？</h2>
          <p>说出你的问题，机器人会用语音回答你。</p>
        </div>
        <article v-for="message in messages" :key="message.id" class="chat-message" :class="{ 'chat-message-user': message.role === 'user' }">
          <span class="chat-speaker">{{ message.role === 'user' ? '你' : '机器人' }}</span>
          <div class="chat-bubble">
            <p v-if="message.text">{{ message.text }}</p>
            <p v-else-if="message.state === 'pending'" class="chat-muted">正在准备回答…</p>
            <p v-else class="chat-muted">{{ message.state === 'interrupted' ? '回复已停止' : '未收到回复' }}</p>
            <span v-if="message.state === 'interrupted' && message.text" class="chat-muted">已打断</span>
          </div>
        </article>
      </div>

      <div class="chat-controls">
        <p v-if="error" class="chat-error" role="alert">{{ error }}</p>
        <form class="chat-composer" :class="{ 'chat-composer-recording': recording }" @submit.prevent="send">
          <Textarea
            v-model="question"
            rows="2"
            auto-resize
            maxlength="1000"
            aria-label="输入问题"
            placeholder="问点什么，或按住麦克风说话…"
            :disabled="!canSend"
            :dt="{ background: 'transparent', borderColor: 'transparent', hoverBorderColor: 'transparent', focusBorderColor: 'transparent', shadow: 'none', focusRing: { shadow: 'none' } }"
            @keydown="composerKeyDown"
          />
          <div class="chat-composer-toolbar">
            <Button
              type="button"
              class="chat-talk-button"
              :label="phase === 'preparing' ? '正在打开麦克风…' : recording ? '松开发送' : '按住说话'"
              icon="pi pi-microphone"
              :severity="recording ? 'danger' : 'secondary'"
              outlined
              rounded
              :disabled="!secureMicrophone || (!canSend && !recording)"
              :aria-pressed="recording"
              :title="secureMicrophone ? '按住说话，松开发送' : '语音输入需要 HTTPS 或 localhost，并允许麦克风权限'"
              @pointerdown="press"
              @pointerup="release"
              @pointercancel="release"
              @lostpointercapture="release"
              @keydown="keyDown"
              @keyup="keyUp"
              @blur="release"
              @contextmenu.prevent
            />
            <span class="chat-status" role="status">{{ transcript || status }}</span>
            <Button v-if="busy" type="button" class="chat-stop-button" aria-label="停止回答" title="停止回答" icon="pi pi-stop" severity="secondary" outlined rounded @click="chat.stopReply" />
            <Button type="submit" class="chat-send-button" icon="pi pi-arrow-up" aria-label="发送" title="发送" rounded :disabled="!canSend || !question.trim()" />
          </div>
        </form>
        <p class="chat-composer-hint">{{ secureMicrophone ? 'Enter 发送 · Shift + Enter 换行' : '文字可直接提问，语音请通过 HTTPS 或 localhost 使用' }}</p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.chat-page {
  --chat-border: color-mix(in srgb, var(--p-surface-400) 16%, transparent);

  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 32rem;
}

.chat-page > .page-header {
  flex-shrink: 0;
}

.chat-connection {
  align-self: center;
  font-size: 0.8125rem;
  color: var(--p-surface-400);
}

.chat-connected {
  color: var(--p-green-400);
}

.chat-panel {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  border: 1px solid var(--chat-border);
  border-radius: 12px;
  background: color-mix(in srgb, var(--p-surface-900) 70%, transparent);
}

.chat-conversation {
  flex: 1;
  min-height: 160px;
  overflow-y: auto;
  padding: 1.5rem;
}

.chat-welcome {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  min-height: 32dvh;
  text-align: center;
  color: var(--p-surface-400);
}

.chat-welcome > i {
  font-size: 2rem;
  color: var(--p-primary-color);
}

.chat-welcome h2 {
  margin: 1rem 0 0;
  color: var(--p-surface-200);
  font-size: 1.25rem;
}

.chat-welcome p {
  font-size: 0.875rem;
}

.chat-message {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 0.375rem;
  margin-bottom: 1.25rem;
}

.chat-speaker {
  font-size: 0.75rem;
  color: var(--p-surface-400);
}

.chat-bubble {
  max-width: min(85%, 48rem);
  padding: 0.875rem 1rem;
  border: 1px solid var(--chat-border);
  border-radius: 10px;
  color: var(--p-surface-200);
}

.chat-bubble p {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  line-height: 1.7;
  font-size: 0.875rem;
}

.chat-message-user {
  align-items: flex-end;
}

.chat-message-user .chat-bubble {
  border-color: var(--p-primary-color);
  background: color-mix(in srgb, var(--p-primary-color) 12%, transparent);
}

.chat-controls {
  flex-shrink: 0;
  width: 100%;
  max-width: 58rem;
  margin: 0 auto;
  padding: 0.5rem 1.75rem 1rem;
}

.chat-composer {
  padding: 0.5rem;
  border: 1px solid var(--chat-border);
  border-radius: 24px;
  background: color-mix(in srgb, var(--p-surface-800) 72%, transparent);
  box-shadow: 0 8px 24px rgb(0 0 0 / 14%);
  transition: border-color 0.2s, box-shadow 0.2s;
}

.chat-composer:focus-within {
  border-color: color-mix(in srgb, var(--p-primary-color) 65%, transparent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--p-primary-color) 8%, transparent);
}

.chat-composer-recording {
  border-color: var(--p-red-400);
}

.chat-composer textarea {
  display: block;
  width: 100%;
  min-height: 4rem;
  max-height: 10rem;
  padding: 0.625rem 0.75rem;
  resize: none;
  overflow-y: auto !important;
  font-size: 0.9375rem;
  line-height: 1.6;
}

.chat-composer-toolbar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0 0.25rem 0.25rem;
}

.chat-status {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--p-surface-400);
  font-size: 0.75rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chat-talk-button {
  flex-shrink: 0;
  touch-action: none;
  user-select: none;
}

.chat-send-button,
.chat-stop-button {
  flex-shrink: 0;
  width: 2.25rem;
  height: 2.25rem;
}

.chat-composer-hint {
  margin: 0.625rem 0 0;
  color: var(--p-surface-400);
  font-size: 0.6875rem;
  line-height: 1.5;
  text-align: center;
}

@media (prefers-reduced-motion: reduce) {
  .chat-composer {
    transition: none;
  }
}

.chat-muted {
  color: var(--p-surface-400);
  font-size: 0.75rem;
}

.chat-error {
  color: var(--p-red-400);
  font-size: 0.8125rem;
  overflow-wrap: anywhere;
}

@media (width <= 768px) {
  .chat-conversation {
    padding: 1rem;
  }

  .chat-controls {
    padding: 0.5rem 0.75rem 0.875rem;
  }

  .chat-bubble {
    max-width: 95%;
  }

}
</style>
