<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import AutoComplete, { type AutoCompleteCompleteEvent } from 'primevue/autocomplete'
import Message from 'primevue/message'
import { listV4l2Nodes, type V4l2Node } from '../../api/system'
import { urlInputMeta, type DeviceFormFields } from '../../forms/deviceForm'

defineProps<{ form: DeviceFormFields }>()

const meta = urlInputMeta('v4l2')
const nodes = ref<V4l2Node[]>([])
const loadFailed = ref(false)
const suggestions = ref<string[]>([])

// Only nodes that capture video are worth offering; a UVC camera's metadata
// node is listed by the server but cannot be used as a source.
const captureNodes = computed(() => nodes.value.filter((node) => node.capture))
const nameByPath = computed(() => new Map(captureNodes.value.map((node) => [node.path, node.name])))
const unreadableCount = computed(() => nodes.value.filter((node) => node.error).length)
const mplaneCount = computed(() => nodes.value.filter((node) => node.mplane).length)

const hint = computed(() => {
  if (loadFailed.value) return '无法读取 nvr 上的节点列表，请直接输入设备路径'
  if (!captureNodes.value.length && mplaneCount.value) {
    return `nvr 上的 ${mplaneCount.value} 个视频节点只支持多平面采集（如 Rockchip rkcif、HDMI RX），FFmpeg 的 V4L2 输入无法读取`
  }
  if (!captureNodes.value.length) return '未在 nvr 上检测到可采集视频的节点，可直接输入设备路径'
  return meta.hint
})

async function loadNodes() {
  try {
    nodes.value = await listV4l2Nodes()
    loadFailed.value = false
  } catch {
    nodes.value = []
    loadFailed.value = true
  }
}

// Match the typed text against the path and the card name.
function search(event: AutoCompleteCompleteEvent) {
  const query = event.query.trim().toLowerCase()
  suggestions.value = captureNodes.value
    .filter(
      (node) =>
        !query || node.path.toLowerCase().includes(query) || node.name.toLowerCase().includes(query),
    )
    .map((node) => node.path)
}

onMounted(loadNodes)
</script>

<template>
  <div class="field">
    <label for="input_value">{{ meta.label }}</label>
    <AutoComplete
      input-id="input_value"
      name="input_value"
      class="field-input"
      dropdown
      :suggestions="suggestions"
      :placeholder="meta.placeholder"
      :invalid="form.input_value?.invalid"
      @complete="search"
    >
      <template #option="{ option }">
        <div class="node-option">
          <span class="mono-text">{{ option }}</span>
          <span class="node-option-name">{{ nameByPath.get(option) }}</span>
        </div>
      </template>
    </AutoComplete>
    <Message v-if="form.input_value?.invalid" severity="error" size="small" variant="simple">
      {{ form.input_value.error?.message }}
    </Message>
    <span class="field-hint">{{ hint }}</span>
    <span v-if="unreadableCount" class="field-hint">
      另有 {{ unreadableCount }} 个节点无法访问（nvr 进程可能没有该设备的读写权限）。
    </span>
  </div>
</template>

<style scoped>
.node-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  width: 100%;
}

.node-option-name {
  color: #94a3b8;
  font-size: 0.75rem;
}
</style>
