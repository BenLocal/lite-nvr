<script setup lang="ts">
import InputNumber from 'primevue/inputnumber'
import Select from 'primevue/select'
import Message from 'primevue/message'
import { onMounted, ref } from 'vue'
import { getMixer } from '../api/audiomixer'
import { useAppToast } from '../utils/toast'
import type { DeviceFormFields } from '../forms/deviceForm'
import { resolutionOptions, videoCodecOptions, outputSupported } from '../forms/outputForm'

defineProps<{ form: DeviceFormFields }>()
const buses = ref<{ label: string; value: string }[]>([{ label: '保持设备原音', value: '' }])
const modeOptions = [{ label: '替换设备原音', value: 'program' }, { label: '叠加设备原音', value: 'add_original' }]
const toast = useAppToast()
onMounted(async () => {
  try { buses.value.push(...(await getMixer()).buses.map(bus => ({ label: bus.id, value: bus.id }))) }
  catch (error) { toast.errorFrom('加载失败', error, '无法获取混音总线') }
})
</script>

<template>
  <div class="field">
    <label for="output_audio_bus_id">混音总线</label>
    <Select input-id="output_audio_bus_id" name="output_audio_bus_id" aria-label="混音总线" :options="buses" option-label="label" option-value="value" class="field-input" />
  </div>
  <div v-if="form.output_audio_bus_id?.value" class="field">
    <label for="output_audio_mode">输出音频组成</label>
    <Select input-id="output_audio_mode" name="output_audio_mode" aria-label="输出音频组成" :options="modeOptions" option-label="label" option-value="value" class="field-input" />
    <span class="field-hint">叠加原音需要开启设备的“包含音频”。编码、采样率和码率在混音总线中设置。</span>
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
