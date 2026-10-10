<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Form from '@primevue/forms/form'
import Button from 'primevue/button'
import Card from 'primevue/card'
import InputText from 'primevue/inputtext'
import Listbox from 'primevue/listbox'
import Message from 'primevue/message'
import ProgressSpinner from 'primevue/progressspinner'
import Textarea from 'primevue/textarea'
import ToggleSwitch from 'primevue/toggleswitch'
import DetectionConfigFields from '../components/DetectionConfigFields.vue'
import GbInputSettings from '../components/device-input/GbInputSettings.vue'
import OnvifInputSettings from '../components/device-input/OnvifInputSettings.vue'
import UrlInputSettings from '../components/device-input/UrlInputSettings.vue'
import V4l2InputSettings from '../components/device-input/V4l2InputSettings.vue'
import X11InputSettings from '../components/device-input/X11InputSettings.vue'
import XiaomiInputSettings from '../components/device-input/XiaomiInputSettings.vue'
import { addDevice, getDevice, updateDevice, type DeviceItem } from '../api/device'
import { getDetectionCapabilities } from '../api/detect'
import {
  buildDevicePayload,
  deviceFormInitialValues,
  inputSettingsKind,
  inputTypeOptions,
  resolveDeviceForm,
  type DeviceFormFields,
} from '../forms/deviceForm'
import { useAppToast } from '../utils/toast'

const route = useRoute()
const router = useRouter()
const appToast = useAppToast()

// /device/new adds a device; /device/:id/edit edits one.
const editId = computed(() => (typeof route.params.id === 'string' ? route.params.id : ''))
const isEdit = computed(() => editId.value !== '')

const device = ref<DeviceItem | null>(null)
const loadingDevice = ref(false)
const saving = ref(false)
const x11Available = ref(false)

// GB28181 device/channel live outside @primevue/forms (see GbInputSettings).
const gbDeviceId = ref('')
const gbChannelId = ref('')

const detectModelOptions = ref<string[]>([])
const detectSupportedInputTypes = ref<string[]>([])
const detectMaxSampleIntervalMs = ref(0)
const detectCapabilityStatus = ref<'loading' | 'ready' | 'error'>('loading')
const detectCapabilityError = ref('')

const formInitialValues = computed(() => deviceFormInitialValues(device.value))

function settingsKind(inputType: unknown) {
  return inputSettingsKind(String(inputType ?? ''))
}

// Set a field from a child component; writing `.value` on the `$form` field
// state is what setFieldValue does internally.
function setFormField(form: DeviceFormFields, name: string, value: unknown) {
  const field = form[name]
  if (field) field.value = value
}

function inputTypeOption(inputType: unknown) {
  return inputTypeOptions.find((option) => option.value === inputType)
}

function inputTypeLabel(inputType: unknown) {
  return inputTypeOption(inputType)?.label ?? '输入'
}

function inputTypeIcon(inputType: unknown) {
  return inputTypeOption(inputType)?.icon ?? 'pi pi-link'
}

function detectSupported(inputType?: unknown): boolean {
  const type = String(inputType ?? '')
  return (
    !!type &&
    (detectCapabilityStatus.value !== 'ready' || detectSupportedInputTypes.value.includes(type))
  )
}

async function loadDetectionCapabilities() {
  detectCapabilityStatus.value = 'loading'
  detectCapabilityError.value = ''
  try {
    const capabilities = await getDetectionCapabilities()
    detectModelOptions.value = [...capabilities.models]
    detectSupportedInputTypes.value = [...capabilities.supported_input_types]
    detectMaxSampleIntervalMs.value = capabilities.max_sample_interval_ms
    detectCapabilityStatus.value = 'ready'
  } catch (error) {
    detectModelOptions.value = []
    detectSupportedInputTypes.value = []
    detectCapabilityError.value = error instanceof Error ? error.message : String(error)
    detectCapabilityStatus.value = 'error'
  }
}

// Split a gb28181 device's input_value JSON into the picker models.
function hydrateGbFields(item: DeviceItem) {
  if (item.input_type !== 'gb28181' || !item.input_value) return
  try {
    const cfg = JSON.parse(item.input_value) as { device_id?: string; channel_id?: string }
    gbDeviceId.value = cfg.device_id ?? ''
    gbChannelId.value = cfg.channel_id ?? ''
  } catch {
    gbDeviceId.value = ''
    gbChannelId.value = ''
  }
}

async function loadDevice() {
  loadingDevice.value = true
  try {
    const found = await getDevice(editId.value)
    if (!found) {
      appToast.warn('设备不存在', `未找到设备 ${editId.value}`)
      await router.replace({ name: 'device' })
      return
    }
    hydrateGbFields(found)
    device.value = found
  } catch (error) {
    appToast.errorFrom('加载失败', error, '设备信息加载失败')
  } finally {
    loadingDevice.value = false
  }
}

onMounted(() => {
  if (isEdit.value) void loadDevice()
  void loadDetectionCapabilities()
})

// The router reuses this component across /device/new and /device/:id/edit,
// so a param change must reload instead of keeping the previous device.
watch(editId, () => {
  device.value = null
  gbDeviceId.value = ''
  gbChannelId.value = ''
  if (isEdit.value) void loadDevice()
})

function resolver(event: { values: Record<string, unknown> }) {
  const inputType = String(event.values.input_type ?? '')
  return resolveDeviceForm(event, {
    gbDeviceId: gbDeviceId.value,
    gbChannelId: gbChannelId.value,
    validateDetection: detectCapabilityStatus.value === 'ready' && detectSupported(inputType),
    maxDetectionSampleIntervalMs: detectMaxSampleIntervalMs.value,
  })
}

async function onSubmit(event: { valid: boolean; values: Record<string, unknown> }) {
  if (!event.valid) return
  const inputType = String(event.values.input_type ?? '')
  if (inputType === 'x11grab' && !x11Available.value) {
    appToast.warn('X11 不可用', '当前环境没有可访问的 X11 显示服务，无法保存该设备')
    return
  }
  if (inputType === 'gb28181' && (!gbDeviceId.value || !gbChannelId.value)) return
  if (Boolean(event.values.detect_enabled) && detectCapabilityStatus.value !== 'ready') {
    appToast.warn('检测能力尚未就绪', '请重试能力加载后再启用检测')
    return
  }
  const payload = buildDevicePayload(event.values, {
    gbDeviceId: gbDeviceId.value,
    gbChannelId: gbChannelId.value,
    supportedDetectionInputTypes: detectSupportedInputTypes.value,
  })
  saving.value = true
  try {
    if (device.value) {
      await updateDevice(device.value.id, payload)
      appToast.success('更新成功', `设备 ${payload.name} 已更新`)
    } else {
      await addDevice(payload)
      appToast.success('添加成功', `设备 ${payload.name} 已添加`)
    }
    await router.push({ name: 'device' })
  } catch (error) {
    appToast.errorFrom('保存失败', error, '设备保存失败')
  } finally {
    saving.value = false
  }
}

// Back returns to wherever the user came from; opened directly (no in-app
// history) it falls back to the device list. Close always goes to the list.
function goBack() {
  if (window.history.state?.back) {
    router.back()
  } else {
    void router.push({ name: 'device' })
  }
}

function close() {
  void router.push({ name: 'device' })
}
</script>

<template>
  <div class="content-section device-form-page">
    <div class="form-page-header">
      <div class="form-page-heading">
        <Button icon="pi pi-arrow-left" text rounded aria-label="返回" title="返回" @click="goBack" />
        <div class="header-content">
          <h1 class="page-title">{{ isEdit ? '编辑设备' : '添加设备' }}</h1>
          <p class="page-subtitle">
            {{ isEdit ? (device?.name ?? '') : '选择输入类型并填写接入参数' }}
          </p>
        </div>
      </div>
      <Button icon="pi pi-times" text rounded aria-label="关闭" title="关闭" @click="close" />
    </div>

    <div v-if="isEdit && !device" class="empty-state device-form-loading">
      <ProgressSpinner v-if="loadingDevice" style="width: 2rem; height: 2rem" stroke-width="4" />
      <p v-else class="empty-state-text">设备信息加载失败</p>
    </div>

    <Form
      v-else
      v-slot="$form"
      :key="device?.id ?? 'new'"
      :resolver="resolver"
      :initial-values="formInitialValues"
      class="device-form-layout"
      @submit="onSubmit"
    >
      <Card class="data-card input-type-card">
        <template #header>
          <div class="card-header">
            <i class="pi pi-list card-header-icon" />
            <span class="card-header-title">输入类型</span>
          </div>
        </template>
        <template #content>
          <Listbox
            name="input_type"
            :options="inputTypeOptions"
            option-label="label"
            option-value="value"
            scroll-height="none"
            class="input-type-list"
            :invalid="$form.input_type?.invalid"
          >
            <template #option="{ option }">
              <div class="input-type-option">
                <i :class="option.icon" />
                <span>{{ option.label }}</span>
              </div>
            </template>
          </Listbox>
          <Message v-if="$form.input_type?.invalid" severity="error" size="small" variant="simple">
            {{ $form.input_type.error?.message }}
          </Message>
        </template>
      </Card>

      <div class="device-form-main">
        <div class="device-form-scroll">
          <Card class="data-card">
            <template #header>
              <div class="card-header">
                <i class="pi pi-id-card card-header-icon" />
                <span class="card-header-title">基本信息</span>
              </div>
            </template>
            <template #content>
              <div class="form-horizontal">
                <div class="field">
                  <label for="name">设备名称</label>
                  <InputText id="name" name="name" class="field-input" :invalid="$form.name?.invalid" />
                  <Message v-if="$form.name?.invalid" severity="error" size="small" variant="simple">
                    {{ $form.name.error?.message }}
                  </Message>
                </div>
              </div>
            </template>
          </Card>

          <Card class="data-card">
            <template #header>
              <div class="card-header">
                <i :class="[inputTypeIcon($form.input_type?.value), 'card-header-icon']" />
                <span class="card-header-title">{{ inputTypeLabel($form.input_type?.value) }} 设置</span>
              </div>
            </template>
            <template #content>
              <div class="form-horizontal">
                <V4l2InputSettings
                  v-if="settingsKind($form.input_type?.value) === 'v4l2'"
                  :form="$form"
                />
                <X11InputSettings
                  v-else-if="settingsKind($form.input_type?.value) === 'x11grab'"
                  :form="$form"
                  @availability="x11Available = $event"
                />
                <XiaomiInputSettings
                  v-else-if="settingsKind($form.input_type?.value) === 'xiaomi'"
                  :form="$form"
                />
                <GbInputSettings
                  v-else-if="settingsKind($form.input_type?.value) === 'gb28181'"
                  v-model:device-id="gbDeviceId"
                  v-model:channel-id="gbChannelId"
                  :form="$form"
                />
                <OnvifInputSettings
                  v-else-if="settingsKind($form.input_type?.value) === 'onvif'"
                  :form="$form"
                  @set-field="(name, value) => setFormField($form, name, value)"
                />
                <UrlInputSettings
                  v-else
                  :form="$form"
                  :input-type="String($form.input_type?.value ?? '')"
                />
              </div>
            </template>
          </Card>

          <Card class="data-card">
            <template #header>
              <div class="card-header">
                <i class="pi pi-sliders-h card-header-icon" />
                <span class="card-header-title">流与录制</span>
              </div>
            </template>
            <template #content>
              <div class="form-horizontal">
                <div class="field">
                  <label for="include_audio">包含音频</label>
                  <ToggleSwitch input-id="include_audio" name="include_audio" />
                  <span class="field-hint">开启后推流会包含音频轨（需输入源带音频）</span>
                </div>
                <div class="field">
                  <label for="record">录制</label>
                  <ToggleSwitch input-id="record" name="record" />
                  <span class="field-hint">开启后该设备的录像会保存到磁盘，可在回放中查看</span>
                </div>
                <div class="field">
                  <label for="description">备注</label>
                  <Textarea id="description" name="description" class="field-input" rows="3" />
                </div>
              </div>
            </template>
          </Card>

          <!-- v-show keeps the detection fields registered for every input type. -->
          <Card v-show="detectSupported($form.input_type?.value)" class="data-card">
            <template #header>
              <div class="card-header">
                <i class="pi pi-eye card-header-icon" />
                <span class="card-header-title">目标检测</span>
              </div>
            </template>
            <template #content>
              <div class="form-horizontal">
                <DetectionConfigFields
                  :form="$form"
                  :model-options="detectModelOptions"
                  :capability-status="detectCapabilityStatus"
                  :capability-error="detectCapabilityError"
                  :max-sample-interval-ms="detectMaxSampleIntervalMs"
                  @retry-capabilities="loadDetectionCapabilities"
                />
              </div>
            </template>
          </Card>
        </div>

        <div class="form-actions">
          <Button type="button" label="取消" severity="secondary" outlined @click="close" />
          <Button type="submit" :label="isEdit ? '保存修改' : '确认添加'" :loading="saving" :disabled="settingsKind($form.input_type?.value) === 'x11grab' && !x11Available" />
        </div>
      </div>
    </Form>
  </div>
</template>

<style scoped>
/* Fill the content area; only the settings column scrolls. */
.device-form-page {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 1rem;
  overflow: hidden;
}

.form-page-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}

.form-page-heading {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}

.device-form-loading {
  min-height: 14rem;
}

.device-form-layout {
  flex: 1 1 auto;
  min-height: 0;
  display: grid;
  grid-template-rows: minmax(0, 1fr);
  grid-template-columns: 14rem minmax(0, 1fr);
  gap: 1rem;
}

.input-type-card {
  align-self: start;
  max-height: 100%;
  overflow-y: auto;
}

.input-type-list {
  width: 100%;
}

.input-type-option {
  display: flex;
  align-items: center;
  gap: 0.625rem;
}

.input-type-option i {
  width: 1rem;
  color: #94a3b8;
}

.device-form-main {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.device-form-scroll {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 1rem;
  overflow-y: auto;
  scrollbar-gutter: stable;
}

/*
 * Right-align the buttons with the inputs: card border (1px) + card content
 * padding (1.25rem). The same stable gutter as the scroll area keeps them
 * aligned when classic scrollbars take width.
 */
.form-actions {
  flex: 0 0 auto;
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  padding: 0.125rem calc(1.25rem + 1px) 0.125rem 0;
  overflow: hidden;
  scrollbar-gutter: stable;
}

/* Single column: the type list and settings stack, so the page scrolls. */
@media (width <= 768px) {
  .device-form-page {
    height: auto;
    overflow: visible;
  }

  .device-form-layout {
    grid-template-rows: none;
    grid-template-columns: minmax(0, 1fr);
  }

  .input-type-card {
    max-height: none;
  }

  .device-form-scroll {
    overflow-y: visible;
  }
}
</style>
