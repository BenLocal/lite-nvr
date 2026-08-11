<script setup lang="ts">
import Button from 'primevue/button'
import InputNumber from 'primevue/inputnumber'
import Message from 'primevue/message'
import MultiSelect from 'primevue/multiselect'
import Slider from 'primevue/slider'
import ToggleSwitch from 'primevue/toggleswitch'

interface FormFieldState {
  value?: unknown
  invalid?: boolean
  error?: { message?: string }
}

const props = defineProps<{
  form: Record<string, FormFieldState | undefined>
  modelOptions: string[]
  capabilityStatus: 'loading' | 'ready' | 'error'
  capabilityError: string
  maxSampleIntervalMs: number
}>()

defineEmits<{ 'retry-capabilities': [] }>()

function confidenceLabel(): string {
  const confidence = Number(props.form.detect_min_confidence?.value ?? 0)
  return Number.isFinite(confidence) ? confidence.toFixed(2) : '—'
}
</script>

<template>
  <div class="field field-inline">
    <label for="detect_enabled">启用检测</label>
    <ToggleSwitch id="detect_enabled" name="detect_enabled" />
    <span class="field-hint">开启后该设备的流会自动跑目标检测</span>
  </div>

  <div class="field">
    <label for="detect_models">检测模型</label>
    <MultiSelect
      id="detect_models"
      name="detect_models"
      class="field-input"
      :options="modelOptions"
      :disabled="capabilityStatus !== 'ready' || modelOptions.length === 0"
      display="chip"
      placeholder="全部模型"
    />
    <span v-if="capabilityStatus === 'loading'" class="field-hint">正在加载检测能力…</span>
    <Message
      v-else-if="capabilityStatus === 'error'"
      severity="error"
      size="small"
      :closable="false"
    >
      检测能力加载失败：{{ capabilityError }}
      <Button label="重试" size="small" text @click="$emit('retry-capabilities')" />
    </Message>
    <span v-else-if="modelOptions.length === 0" class="field-hint">
      未配置检测模型（缺少 models.json）。
    </span>
    <span v-else class="field-hint">留空表示运行全部已配置模型。</span>
  </div>

  <div class="field-grid">
    <div class="field">
      <label for="detect_sample_every_ms">抽帧间隔 (ms)</label>
      <InputNumber
        id="detect_sample_every_ms"
        name="detect_sample_every_ms"
        class="field-input"
        :min="0"
        :max="maxSampleIntervalMs"
        :step="100"
        :invalid="form.detect_sample_every_ms?.invalid"
        show-buttons
      />
      <Message
        v-if="form.detect_sample_every_ms?.invalid"
        severity="error"
        size="small"
        variant="simple"
      >
        {{ form.detect_sample_every_ms?.error?.message }}
      </Message>
      <span class="field-hint">0 表示使用服务端默认间隔。</span>
    </div>
    <div class="field">
      <label for="detect_min_confidence">置信度下限：{{ confidenceLabel() }}</label>
      <Slider
        id="detect_min_confidence"
        name="detect_min_confidence"
        class="field-input"
        :min="0"
        :max="1"
        :step="0.05"
      />
      <Message
        v-if="form.detect_min_confidence?.invalid"
        severity="error"
        size="small"
        variant="simple"
      >
        {{ form.detect_min_confidence?.error?.message }}
      </Message>
      <span class="field-hint">0 表示沿用每个模型自带的阈值。</span>
    </div>
  </div>
</template>
