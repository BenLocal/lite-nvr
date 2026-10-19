import { expect, test, type Page, type WebSocketRoute } from '@playwright/test'

test.use({
  permissions: ['microphone'],
  launchOptions: {
    executablePath: process.env.PLAYWRIGHT_EXECUTABLE_PATH,
    args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream'],
  },
})

async function mockChat(page: Page) {
  const frames: number[] = []
  const state = { frames, stopped: false, disconnected: false }
  await page.addInitScript(() => {
    localStorage.setItem('nvr-auth-token', 'test-chat-token')
    const getUserMedia = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices)
    navigator.mediaDevices.getUserMedia = async constraints => {
      const stream = await getUserMedia(constraints)
      Reflect.set(window, 'chatMicrophoneTracks', stream.getTracks())
      return stream
    }
  })
  await page.routeWebSocket('**/socket.io/**', ws => {
    let turn = 0
    let awaitingPcm = false
    function event(name: string, data: unknown): void {
      ws.send(`42/chat,${JSON.stringify([name, data])}`)
    }
    function reply(text: string, done = true): void {
      turn++
      event('reply_delta', { turn, text })
      // Exercise actual binary attachment decoding and Web Audio playback.
      const pcm = Buffer.alloc(16000 * 2)
      ws.send(`451-/chat,${JSON.stringify(['reply_audio', {
        turn, seq: 0, sample_rate: 16000, pcm: { _placeholder: true, num: 0 },
      }])}`)
      ws.send(pcm)
      if (done) event('reply_done', { turn, interrupted: false })
    }
    ws.onClose(() => { state.disconnected = true })
    ws.onMessage(message => {
      if (typeof message !== 'string') {
        if (awaitingPcm) state.frames.push(message.length)
        awaitingPcm = false
        return
      }
      if (message.startsWith('40/chat,')) {
        expect(JSON.parse(message.slice('40/chat,'.length))).toEqual({ token: 'test-chat-token' })
        ws.send('40/chat,{"sid":"chat-session"}')
      } else if (message.startsWith('451-/chat,')) {
        awaitingPcm = true
      } else if (message.startsWith('42/chat,')) {
        const [name, text] = JSON.parse(message.slice('42/chat,'.length))
        if (name === 'text') reply(text === '长回复' ? '这是一段正在播放的长回复。' : '你好！有什么可以帮你？', text !== '长回复')
        if (name === 'cancel' && turn) event('reply_done', { turn, interrupted: true })
        if (name === 'stop') {
          state.stopped = true
          event('asr_final', { text: '北京有哪些景点？' })
          reply('你可以去故宫和颐和园。')
        }
      }
    })
    ws.send('0{"sid":"chat-engine","upgrades":[],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}')
  })
  return state
}

test('chat sidebar, typed replies, interruption, real microphone frames and reset', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  const state = await mockChat(page)
  await page.goto('/nvr/chat')
  await expect(page.getByRole('link', { name: '问答机器人' })).toBeVisible()
  await expect(page.getByText('可以开始对话', { exact: true })).toBeVisible()

  const question = page.getByRole('textbox', { name: '输入问题' })
  await question.fill('你好')
  await page.getByRole('button', { name: '发送', exact: true }).click()
  await expect(page.getByRole('log')).toContainText('你好！有什么可以帮你？')
  await expect(page.getByText('机器人正在说话', { exact: true })).toBeVisible()

  await question.fill('长回复')
  await question.press('Enter')
  await expect(page.getByRole('log')).toContainText('这是一段正在播放的长回复。')
  await page.getByRole('button', { name: '停止回答' }).click()
  await expect(page.getByRole('button', { name: '停止回答' })).toHaveCount(0)

  const talk = page.getByRole('button', { name: '按住说话', exact: true })
  await talk.hover()
  await page.mouse.down()
  await expect(page.getByRole('button', { name: '松开发送', exact: true })).toBeVisible()
  await expect.poll(() => state.frames.length).toBeGreaterThan(0)
  await page.mouse.up()
  await expect.poll(() => state.stopped).toBe(true)
  await expect(page.getByRole('log')).toContainText('北京有哪些景点？')
  await expect(page.getByRole('log')).toContainText('你可以去故宫和颐和园。')
  expect(state.frames.every(size => size > 0 && size <= 3200 && size % 2 === 0)).toBe(true)
  expect(await page.evaluate(() => {
    const tracks: unknown = Reflect.get(window, 'chatMicrophoneTracks')
    return Array.isArray(tracks) && tracks.length > 0 && tracks.every(track => track instanceof MediaStreamTrack && track.readyState === 'ended')
  })).toBe(true)
  await page.screenshot({ path: '/tmp/lite-nvr-chat-desktop.png', fullPage: true })

  await page.getByRole('button', { name: '新建对话' }).click()
  await expect(page.getByText('想聊些什么？')).toBeVisible()
  await page.setViewportSize({ width: 390, height: 844 })
  await page.getByRole('button', { name: '切换侧边栏' }).click()
  await expect.poll(async () => {
    const sidebar = await page.locator('aside').boundingBox()
    return sidebar ? sidebar.x + sidebar.width : 0
  }).toBeLessThanOrEqual(1)
  await expect(talk).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await page.screenshot({ path: '/tmp/lite-nvr-chat-mobile.png', fullPage: true })

  // Leave while the microphone is open: the route must release capture and
  // tear down the Socket.IO conversation, even without a key-up event.
  await talk.focus()
  await page.keyboard.down('Space')
  await expect(page.getByRole('button', { name: '松开发送', exact: true })).toBeVisible()
  // Keep the destination's unrelated API requests away from a real NVR.
  await page.route(url => url.pathname.startsWith('/api/'), route => route.fulfill({
    status: 503,
    json: { code: 1, message: 'Service unavailable in this UI test', data: null },
  }))
  await page.getByRole('button', { name: '切换侧边栏' }).click()
  await page.getByRole('link', { name: '设置' }).click()
  await expect(page).toHaveURL(/\/nvr\/settings$/)
  await page.keyboard.up('Space')
  await expect.poll(() => state.disconnected).toBe(true)
  expect(await page.evaluate(() => {
    const tracks: unknown = Reflect.get(window, 'chatMicrophoneTracks')
    return Array.isArray(tracks) && tracks.length > 0 && tracks.every(track => track instanceof MediaStreamTrack && track.readyState === 'ended')
  })).toBe(true)
  expect(errors).toEqual([])
})

test('unavailable service gives a useful message and offers an explicit reconnect', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nvr-auth-token', 'test-chat-token'))
  await page.routeWebSocket('**/socket.io/**', (ws: WebSocketRoute) => {
    ws.onMessage(message => {
      if (typeof message === 'string' && message.startsWith('40/chat,')) {
        ws.send('44/chat,{"message":"Invalid namespace"}')
      }
    })
    ws.send('0{"sid":"no-chat","upgrades":[],"pingInterval":25000,"pingTimeout":20000}')
  })
  await page.goto('/nvr/chat')
  await expect(page.getByRole('alert')).toContainText('请确认主机已启用语音对话服务')
  await expect(page.getByRole('button', { name: '重新连接' })).toBeVisible()
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeDisabled()
})
