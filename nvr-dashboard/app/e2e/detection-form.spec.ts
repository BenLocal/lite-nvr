import { expect, test } from '@playwright/test'

test('detection capabilities failure is retryable and not reported as missing models', async ({
  page,
}) => {
  let capabilityCalls = 0
  await page.addInitScript(() => localStorage.setItem('nvr-auth-token', 'e2e-token'))
  await page.route('**/api/device/list', (route) =>
    route.fulfill({ json: { code: 0, message: 'success', data: [] } }),
  )
  await page.route('**/api/gb/streams', (route) =>
    route.fulfill({ json: { code: 0, message: 'success', data: [] } }),
  )
  await page.route('**/api/detect/capabilities', (route) => {
    capabilityCalls++
    if (capabilityCalls === 1) {
      return route.fulfill({ status: 503, body: 'temporarily unavailable' })
    }
    return route.fulfill({
      json: {
        models: ['person'],
        supported_input_types: ['rtsp'],
        max_sample_interval_ms: 3_600_000,
      },
    })
  })

  await page.goto('/nvr/device')
  await page.getByRole('button', { name: '添加设备' }).click()
  await page.getByRole('tab', { name: '检测' }).click()

  await expect(page.getByText(/检测能力加载失败/)).toBeVisible()
  await expect(page.getByText(/缺少 models\.json/)).toHaveCount(0)
  await page.getByRole('button', { name: '重试' }).click()

  await expect(page.getByText('留空表示运行全部已配置模型。')).toBeVisible()
  expect(capabilityCalls).toBe(2)
})

test('adding an RTSP device submits validated detection config', async ({ page }) => {
  let submitted: Record<string, unknown> | undefined
  await page.addInitScript(() => localStorage.setItem('nvr-auth-token', 'e2e-token'))
  await page.route('**/api/device/list', (route) =>
    route.fulfill({ json: { code: 0, message: 'success', data: [] } }),
  )
  await page.route('**/api/gb/streams', (route) =>
    route.fulfill({ json: { code: 0, message: 'success', data: [] } }),
  )
  await page.route('**/api/detect/capabilities', (route) =>
    route.fulfill({
      json: {
        models: ['person'],
        supported_input_types: ['rtsp'],
        max_sample_interval_ms: 3_600_000,
      },
    }),
  )
  await page.route('**/api/device/add', async (route) => {
    submitted = route.request().postDataJSON() as Record<string, unknown>
    await route.fulfill({ json: { code: 0, message: 'success', data: submitted } })
  })

  await page.goto('/nvr/device')
  await page.getByRole('button', { name: '添加设备' }).click()
  await page.locator('input[name="name"]').fill('门口摄像头')
  await page.locator('input[name="input_value"]').fill('rtsp://camera/live')
  await page.getByRole('tab', { name: '检测' }).click()
  await page.getByRole('switch').check()
  await page.getByRole('button', { name: '确认添加' }).click()

  await expect.poll(() => submitted).toBeTruthy()
  expect(submitted).toMatchObject({
    name: '门口摄像头',
    input_type: 'rtsp',
    input_value: 'rtsp://camera/live',
    config: {
      detect: {
        enabled: true,
        models: [],
        sample_every_ms: 1_000,
        min_confidence: 0,
      },
    },
  })
})
