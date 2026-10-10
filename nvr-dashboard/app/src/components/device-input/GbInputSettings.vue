<script setup lang="ts">
import { onMounted, ref } from 'vue'
import Message from 'primevue/message'
import Select from 'primevue/select'
import Tag from 'primevue/tag'
import { getGbCatalog, getGbDevices, type GbChannel, type GbDevice } from '../../api/gb'
import type { DeviceFormFields } from '../../forms/deviceForm'

// The device/channel pickers are standalone v-models (not @primevue/forms
// fields) because their options load on demand from the live registrar; the
// page resolves them into input_value.
defineProps<{ form: DeviceFormFields }>()

const deviceId = defineModel<string>('deviceId', { required: true })
const channelId = defineModel<string>('channelId', { required: true })

const devices = ref<GbDevice[]>([])
const channels = ref<GbChannel[]>([])

async function loadDevices() {
  try {
    devices.value = await getGbDevices()
  } catch {
    devices.value = []
  }
}

async function loadCatalog(id: string) {
  try {
    const list = await getGbCatalog(id)
    // Drop a stale response if the user switched devices meanwhile, so device
    // A's channels never show under device B.
    if (deviceId.value === id) channels.value = list
  } catch {
    if (deviceId.value === id) channels.value = []
  }
}

function onDeviceChange(id: string) {
  deviceId.value = id
  channelId.value = ''
  channels.value = []
  if (id) void loadCatalog(id)
}

// Editing: the saved device/channel arrive as initial model values, so load
// the catalog for them without clearing the saved channel.
onMounted(() => {
  void loadDevices()
  if (deviceId.value) void loadCatalog(deviceId.value)
})
</script>

<template>
  <div class="field">
    <label for="gb_device">国标设备</label>
    <Select
      id="gb_device"
      class="field-input"
      :options="devices"
      option-label="device_id"
      option-value="device_id"
      :model-value="deviceId"
      placeholder="选择已注册的国标设备"
      @update:model-value="onDeviceChange"
      @before-show="loadDevices"
    >
      <template #option="{ option }">
        <div class="gb-option">
          <span class="mono-text">{{ option.device_id }}</span>
          <Tag v-if="!option.online" value="离线" severity="secondary" />
        </div>
      </template>
    </Select>
  </div>
  <div class="field">
    <label for="gb_channel">国标通道</label>
    <Select
      id="gb_channel"
      v-model="channelId"
      class="field-input"
      :options="channels"
      option-label="name"
      option-value="channel_id"
      placeholder="选择通道"
      :disabled="!deviceId"
    />
    <Message v-if="form.input_value?.invalid" severity="error" size="small" variant="simple">
      {{ form.input_value.error?.message }}
    </Message>
  </div>
  <div class="field">
    <span class="field-hint">
      国标设备需先注册到本平台（NVR_GB_ENABLE=1）。下拉框列出已注册设备（离线设备会标注），
      选择后加载其通道，保存后仅在有人观看时按需 INVITE 拉流。
    </span>
  </div>
</template>

<style scoped>
.gb-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  width: 100%;
}
</style>
