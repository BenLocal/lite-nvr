<script setup lang="ts">
import InputNumber from 'primevue/inputnumber'
import Select from 'primevue/select'
import Message from 'primevue/message'
import type { DeviceFormFields } from '../forms/deviceForm'
import { resolutionOptions, videoCodecOptions, audioCodecOptions, sampleRateOptions, outputSupported } from '../forms/outputForm'

defineProps<{ form: DeviceFormFields }>()
</script>

<template>
  <Message v-if="!outputSupported(form.input_type?.value)" severity="info" size="small">
    此设备使用原生转发，暂不支持输出转码参数。
  </Message>
  <div v-show="outputSupported(form.input_type?.value)" class="form-horizontal">
    <div class="field">
      <label for="output_video_codec">视频编码</label>
      <Select input-id="output_video_codec" name="output_video_codec" aria-label="视频编码" placeholder="保持默认" :options="videoCodecOptions" option-label="label" option-value="value" class="field-input" />
    </div>
    <div class="field">
      <label for="output_resolution">输出分辨率</label>
      <Select input-id="output_resolution" name="output_resolution" aria-label="输出分辨率" placeholder="保持输入分辨率" :options="resolutionOptions" option-label="label" option-value="value" class="field-input" />
      <span class="field-hint">保持比例向下缩放，不会放大低分辨率输入。</span>
    </div>
    <div v-for="field in [
      { name: 'output_fps', label: '视频 FPS', min: 1, max: 120, suffix: '' },
      { name: 'output_max_bitrate', label: '最大视频码率', min: 32, max: 100000, suffix: ' Kbps' },
    ]" :key="field.name" class="field">
      <label :for="field.name">{{ field.label }}</label>
      <InputNumber :input-id="field.name" :name="field.name" :min="field.min" :max="field.max" :suffix="field.suffix" :use-grouping="false" placeholder="保持默认" class="field-input" :invalid="form[field.name]?.invalid" />
      <Message v-if="form[field.name]?.invalid" severity="error" size="small" variant="simple">{{ form[field.name]?.error?.message }}</Message>
    </div>
    <div class="field">
      <label for="output_audio_codec">音频编码</label>
      <Select input-id="output_audio_codec" name="output_audio_codec" aria-label="音频编码" placeholder="保持默认" :options="audioCodecOptions" option-label="label" option-value="value" class="field-input" />
      <span class="field-hint">音频设置仅在开启“包含音频”且输入有音轨时生效。</span>
    </div>
    <div class="field">
      <label for="output_sample_rate">音频采样率</label>
      <Select input-id="output_sample_rate" name="output_sample_rate" aria-label="音频采样率" placeholder="保持输入采样率" :options="sampleRateOptions(form.output_audio_codec?.value)" option-label="label" option-value="value" class="field-input" :invalid="form.output_sample_rate?.invalid" />
      <Message v-if="form.output_sample_rate?.invalid" severity="error" size="small" variant="simple">{{ form.output_sample_rate.error?.message }}</Message>
    </div>
  </div>
</template>
