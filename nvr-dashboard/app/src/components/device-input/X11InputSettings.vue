<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import AutoComplete, { type AutoCompleteCompleteEvent } from 'primevue/autocomplete'
import Message from 'primevue/message'
import { listX11Displays, type X11Display } from '../../api/system'
import { urlInputMeta, type DeviceFormFields } from '../../forms/deviceForm'

defineProps<{ form: DeviceFormFields }>()
const emit = defineEmits<{ availability: [available: boolean] }>()

const meta = urlInputMeta('x11grab')
const displays = ref<X11Display[]>([])
const loadFailed = ref(false)
const suggestions = ref<string[]>([])
const available = ref(false)
const reason = ref('正在检查 X11 显示环境…')

const currentDisplay = computed(() => displays.value.find((d) => d.current)?.display)

const hint = computed(() =>
  loadFailed.value ? '无法检查 X11 显示环境，暂时不能添加 X11 设备' : reason.value || meta.hint,
)

async function loadDisplays() {
  emit('availability', false)
  try {
    const environment = await listX11Displays()
    displays.value = environment.displays
    available.value = environment.available
    reason.value = environment.reason
    emit('availability', available.value)
    loadFailed.value = false
  } catch {
    displays.value = []
    available.value = false
    emit('availability', false)
    loadFailed.value = true
  }
}

function search(event: AutoCompleteCompleteEvent) {
  const query = event.query.trim().toLowerCase()
  suggestions.value = displays.value
    .map((d) => d.display)
    .filter((display) => !query || display.toLowerCase().includes(query))
}

onMounted(loadDisplays)
</script>

<template>
  <div class="field">
    <label for="input_value">{{ meta.label }}</label>
    <AutoComplete
      input-id="input_value"
      name="input_value"
      class="field-input"
      dropdown
      :disabled="!available"
      :suggestions="suggestions"
      :placeholder="meta.placeholder"
      :invalid="form.input_value?.invalid"
      @complete="search"
    >
      <template #option="{ option }">
        <div class="display-option">
          <span class="mono-text">{{ option }}</span>
          <span v-if="option === currentDisplay" class="display-option-current">nvr 当前 DISPLAY</span>
        </div>
      </template>
    </AutoComplete>
    <Message v-if="form.input_value?.invalid" severity="error" size="small" variant="simple">
      {{ form.input_value.error?.message }}
    </Message>
    <span class="field-hint">{{ hint }}</span>
  </div>
</template>

<style scoped>
.display-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  width: 100%;
}

.display-option-current {
  color: #94a3b8;
  font-size: 0.75rem;
}
</style>
