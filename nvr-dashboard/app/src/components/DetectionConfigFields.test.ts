import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import DetectionConfigFields from './DetectionConfigFields.vue'

describe('DetectionConfigFields capability states', () => {
  it('shows a retryable request error instead of a missing-model message', async () => {
    const wrapper = mount(DetectionConfigFields, {
      props: {
        form: {},
        modelOptions: [],
        capabilityStatus: 'error',
        capabilityError: '网络不可用',
        maxSampleIntervalMs: 3_600_000,
      },
      global: {
        stubs: {
          ToggleSwitch: true,
          MultiSelect: true,
          InputNumber: true,
          Slider: true,
          Message: { template: '<div><slot /></div>' },
          Button: {
            emits: ['click'],
            template: '<button type="button" @click="$emit(\'click\')">重试</button>',
          },
        },
      },
    })

    expect(wrapper.text()).toContain('检测能力加载失败：网络不可用')
    expect(wrapper.text()).not.toContain('缺少 models.json')
    await wrapper.get('button').trigger('click')
    expect(wrapper.emitted('retry-capabilities')).toHaveLength(1)
  })
})
