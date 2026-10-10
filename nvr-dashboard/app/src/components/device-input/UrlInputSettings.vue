<script setup lang="ts">
import { computed } from 'vue'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import { urlInputMeta, type DeviceFormFields } from '../../forms/deviceForm'

// Settings for input types that take a single address: rtsp, rtmp, stream,
// file, v4l2, x11grab, lavfi.
const props = defineProps<{
  form: DeviceFormFields
  inputType: string
}>()

const meta = computed(() => urlInputMeta(props.inputType))
</script>

<template>
  <div class="field">
    <label for="input_value">{{ meta.label }}</label>
    <InputText
      id="input_value"
      name="input_value"
      class="field-input"
      :placeholder="meta.placeholder"
      :invalid="form.input_value?.invalid"
    />
    <Message v-if="form.input_value?.invalid" severity="error" size="small" variant="simple">
      {{ form.input_value.error?.message }}
    </Message>
    <span v-if="meta.hint" class="field-hint">{{ meta.hint }}</span>
  </div>
</template>
