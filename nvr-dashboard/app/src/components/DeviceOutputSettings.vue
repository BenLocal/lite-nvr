<script setup lang="ts">
import InputNumber from 'primevue/inputnumber'
import Select from 'primevue/select'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import { onMounted, ref } from 'vue'
import { getMixer } from '../api/audiomixer'
import { useAppToast } from '../utils/toast'
import type { DeviceFormFields } from '../forms/deviceForm'
import { resolutionOptions, videoCodecOptions, outputSupported } from '../forms/outputForm'

defineProps<{ form: DeviceFormFields }>()
const mixBusId = ref('')
const toast = useAppToast()
onMounted(async () => {
  try { mixBusId.value = (await getMixer()).buses[0]?.id ?? '' }
  catch (error) { toast.errorFrom('加载失败', error, '无法获取混音总线') }
})
</script>

<template>
  <div class="field">
    <div class="audio-bus-toggle">
      <label for="output-audio-mix">混音总线</label>
      <ToggleSwitch input-id="output-audio-mix" name="output_audio_bus_id" :true-value="mixBusId || null" false-value="" :disabled="!mixBusId" />
      <span>{{ form.output_audio_bus_id?.value ? '启用' : '关闭' }}</span>
    </div>
    <span v-if="!mixBusId" class="field-hint">请先在混音台配置音频输入。</span>
  </div>
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

  </div>
</template>

<style scoped>
.audio-bus-toggle {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.audio-bus-toggle label {
  margin: 0;
}
</style>
