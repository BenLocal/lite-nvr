<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Card from 'primevue/card'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import Button from 'primevue/button'
import Dialog from 'primevue/dialog'
import InputText from 'primevue/inputtext'
import Tag from 'primevue/tag'
import Select from 'primevue/select'
import Divider from 'primevue/divider'
import { Form } from '@primevue/forms'
import ConfirmDialog from 'primevue/confirmdialog'
import { useConfirm } from 'primevue/useconfirm'
import { useToast } from 'primevue/usetoast'
import flvjs from 'flv.js'

interface Device {
  id: number
  name: string
  status: string
  input: any
  output: any
  created_at?: string
  updated_at?: string
}

const createDefaultOutput = (streamName = 'stream') => ({
  t: 'zlm',
  zlm: {
    app: 'live',
    stream: streamName
  },
  encode: {
    preset: '',
    bitrate: null
  }
})

const normalizeOutput = (output: any, name = '') => {
  const stream = output?.zlm?.stream || name || 'stream'
  return {
    ...createDefaultOutput(stream),
    ...output,
    t: 'zlm',
    zlm: {
      app: output?.zlm?.app || 'live',
      stream
    },
    encode: {
      preset: output?.encode?.preset || '',
      bitrate: output?.encode?.bitrate ?? null
    }
  }
}

const devices = ref<Device[]>([])
const loading = ref(true) // Changed default loading to true
const showDialog = ref(false)
const showPreviewDialog = ref(false)
const isEdit = ref(false)
const submitting = ref(false)
const previewDevice = ref<Device | null>(null)
const previewVideoRef = ref<HTMLVideoElement | null>(null)
const previewLoading = ref(false)
const previewError = ref('')
let flvPlayer: flvjs.Player | null = null
const formData = ref({
  id: 0,
  name: '',
  input: {
    t: 'net',
    i: ''
  },
  output: createDefaultOutput()
})

const inputOptions = [
  { label: 'Network 流 (Rtsp/Rtmp/HTTP)', value: 'net' },
  { label: '本地设备 (v4l2/DirectShow)', value: 'v4l2' },
  { label: '屏幕录制 (x11grab/gdigrab)', value: 'x11grab' }
]

const confirm = useConfirm()
const toast = useToast()

const showAlert = (message: string, severity: 'success' | 'info' | 'warn' | 'error' = 'info') => {
  toast.add({ severity, summary: '提示', detail: message, life: 3000 })
}

const fetchDevices = async () => {
  loading.value = true
  try {
    const res = await fetch('/api/device')
    if (res.ok) {
      devices.value = await res.json()
    } else {
      console.error('Failed to fetch devices')
      showAlert('获取设备列表失败', 'error')
    }
  } catch (error) {
    console.error('Error fetching devices', error)
    showAlert('获取设备列表发生错误', 'error')
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchDevices()
})

const resetForm = () => {
  formData.value = {
    id: 0,
    name: '',
    input: {
      t: 'net',
      i: ''
    },
    output: createDefaultOutput()
  }
}

const openAddDevice = () => {
  isEdit.value = false
  resetForm()
  showDialog.value = true
}

const openEditDevice = (device: Device) => {
  isEdit.value = true
  // Deep copy the input/output so modifications don't instantly reflect
  formData.value = {
    id: device.id,
    name: device.name,
    input: device.input ? JSON.parse(JSON.stringify(device.input)) : { t: 'net', i: '' },
    output: normalizeOutput(
      device.output ? JSON.parse(JSON.stringify(device.output)) : null,
      device.name
    )
  }
  showDialog.value = true
}

const saveDevice = async () => {
  if (!formData.value.name || !formData.value.input.i) return

  formData.value.output = normalizeOutput(formData.value.output, formData.value.name)

  submitting.value = true
  try {
    const url = isEdit.value ? `/api/device/${formData.value.id}` : '/api/device'
    const method = isEdit.value ? 'PUT' : 'POST' // Changed method for edit

    const res = await fetch(url, {
      method,
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        name: formData.value.name,
        input: formData.value.input,
        output: formData.value.output
      })
    })

    if (res.ok) {
      showDialog.value = false
      fetchDevices()
      showAlert(isEdit.value ? '设备修改成功' : '设备添加成功', 'success')
    } else {
      const errorText = await res.text()
      showAlert(`保存失败: ${errorText}`, 'error')
    }
  } catch (error) {
    showAlert('发生网络错误，请稍后重试', 'error')
  } finally {
    submitting.value = false
  }
}

const confirmDelete = (id: number) => {
  confirm.require({
    message: '确定要删除此设备吗？',
    header: '确认删除',
    icon: 'pi pi-exclamation-triangle',
    rejectProps: {
      label: '取消',
      severity: 'secondary',
      outlined: true
    },
    acceptProps: {
      label: '确定',
      severity: 'danger'
    },
    accept: async () => {
      try {
        const res = await fetch(`/api/device/${id}/delete`, { method: 'POST' })
        if (res.ok) {
          fetchDevices()
          showAlert('设备已删除', 'success')
        } else {
          const errorText = await res.text()
          showAlert(`删除失败: ${errorText}`, 'error')
        }
      } catch (error) {
        showAlert('发生网络错误，请稍后重试', 'error')
      }
    }
  })
}

const getStatusSeverity = (status: string) => {
  switch (status.toLowerCase()) {
    case 'online':
    case 'playing':
      return 'success'
    case 'offline':
    case 'error':
      return 'danger'
    default:
      return 'info'
  }
}

const getPreviewFlvUrl = (device: Device) => {
  const app = device.output?.zlm?.app || 'live'
  const stream = device.output?.zlm?.stream || device.name || 'stream'
  const hostname = window.location.hostname || '127.0.0.1'
  return `http://${hostname}:8553/${encodeURIComponent(app)}/${encodeURIComponent(stream)}.live.flv`
}

const destroyPreviewPlayer = () => {
  if (flvPlayer) {
    flvPlayer.pause()
    flvPlayer.unload()
    flvPlayer.detachMediaElement()
    flvPlayer.destroy()
    flvPlayer = null
  }
}

const initPreviewPlayer = async () => {
  if (!showPreviewDialog.value || !previewDevice.value) return

  await nextTick()

  const video = previewVideoRef.value
  if (!video) return

  destroyPreviewPlayer()
  previewError.value = ''

  if (!flvjs.isSupported()) {
    previewError.value = '当前浏览器不支持 FLV 直播预览'
    return
  }

  previewLoading.value = true
  const previewUrl = getPreviewFlvUrl(previewDevice.value)
  flvPlayer = flvjs.createPlayer(
    {
      type: 'flv',
      isLive: true,
      url: previewUrl
    },
    {
      enableStashBuffer: false,
      stashInitialSize: 128
    }
  )

  flvPlayer.attachMediaElement(video)
  flvPlayer.load()

  flvPlayer.on(flvjs.Events.LOADING_COMPLETE, () => {
    previewLoading.value = false
  })
  flvPlayer.on(flvjs.Events.ERROR, (_errorType, _errorDetail, errorInfo) => {
    previewLoading.value = false
    previewError.value = `预览失败: ${errorInfo?.msg || 'FLV 流连接异常'}`
  })

  try {
    await video.play()
    previewLoading.value = false
  } catch (error) {
    previewLoading.value = false
    previewError.value = '浏览器阻止了自动播放，请手动点击画面播放'
  }
}

const openPreview = (device: Device) => {
  previewDevice.value = device
  previewError.value = ''
  previewLoading.value = true
  showPreviewDialog.value = true
}

const closePreview = () => {
  showPreviewDialog.value = false
  previewDevice.value = null
  previewLoading.value = false
  previewError.value = ''
  destroyPreviewPlayer()
}

watch(showPreviewDialog, (visible) => {
  if (visible) {
    initPreviewPlayer()
  } else {
    destroyPreviewPlayer()
  }
})

onBeforeUnmount(() => {
  destroyPreviewPlayer()
})
</script>

<template>
  <div class="content-section">
    <ConfirmDialog></ConfirmDialog>
    <Card class="content-card">
      <template #title>设备管理</template>
      <template #subtitle>管理接入的 NVR 设备列表。</template>
      <template #content>
        <div class="flex justify-end mb-4">
          <Button label="添加设备" icon="pi pi-plus" @click="openAddDevice" />
        </div>

        <DataTable :value="devices" :loading="loading" striped-rows class="content-table" responsive-layout="scroll">
          <Column field="id" header="ID" headerStyle="width: 5rem"></Column>
          <Column field="name" header="设备名称"></Column>
          <Column header="流地址">
            <template #body="slotProps">
              <span>{{ slotProps.data.input?.i || '' }}</span>
            </template>
          </Column>
          <Column header="状态" headerStyle="width: 8rem">
            <template #body="slotProps">
              <Tag :value="slotProps.data.status" :severity="getStatusSeverity(slotProps.data.status)" />
            </template>
          </Column>
          <Column field="created_at" header="接入时间"></Column>
          <Column header="操作" :exportable="false" style="min-width: 16rem">
            <template #body="slotProps">
              <Button icon="pi pi-video" outlined rounded class="mr-2" @click="openPreview(slotProps.data)" />
              <Button icon="pi pi-pencil" outlined rounded class="mr-2" @click="openEditDevice(slotProps.data)" />
              <Button icon="pi pi-trash" outlined rounded severity="danger" @click="confirmDelete(slotProps.data.id)" />
            </template>
          </Column>
          <template #empty>
            <div class="empty-message">暂无设备数据</div>
          </template>
        </DataTable>
      </template>
    </Card>

    <Dialog v-model:visible="showDialog" :style="{ width: '45rem' }" :header="isEdit ? '编辑设备' : '添加设备'" :modal="true">
      <span class="form-subtitle">
        {{ isEdit ? '修改接入设备的详细信息。' : '配置输入和输出管线。' }}
      </span>
      <Form @submit="saveDevice" class="form-container">
        <!-- Input Section -->
        <h3 class="text-lg font-bold mb-0">输入源配置 (Input)</h3>
        <div class="form-row">
          <label for="name" class="form-label">设备名称</label>
          <InputText id="name" name="name" v-model.trim="formData.name" required="true" class="form-input" autofocus autocomplete="off" placeholder="例如: 办公室大门" />
        </div>
        <div class="form-row">
          <label for="inputType" class="form-label">输入类型</label>
          <Select id="inputType" v-model="formData.input.t" :options="inputOptions" optionLabel="label" optionValue="value" class="form-input" />
        </div>
        <div class="form-row">
          <label for="url" class="form-label">输入地址</label>
          <InputText id="url" name="url" v-model.trim="formData.input.i" required="true" class="form-input" autocomplete="off" placeholder="rtsp://... 或者是 /dev/video0" />
        </div>

        <Divider />

        <h3 class="text-lg font-bold mb-0">输出流配置 (Output)</h3>
        <div class="output-card">
          <div class="form-row">
            <label class="form-label">输出类型</label>
            <InputText value="ZLM 推流" class="form-input" disabled />
          </div>

          <div class="form-row">
            <label class="form-label">ZLM App</label>
            <InputText v-model.trim="formData.output.zlm.app" class="form-input" placeholder="live" />
          </div>
          <div class="form-row">
            <label class="form-label">ZLM Stream</label>
            <InputText v-model.trim="formData.output.zlm.stream" class="form-input" placeholder="stream" />
          </div>

          <div class="output-divider">
            <span class="output-divider-title">高级编码选项 (留空则使用默认配置)</span>
            <div class="form-row">
              <label class="form-label">x264 Preset</label>
              <InputText v-model.trim="formData.output.encode.preset" class="form-input p-inputtext-sm" placeholder="ultrafast, superfast..." />
            </div>
            <div class="form-row">
              <label class="form-label">目标码率(bps)</label>
              <InputText v-model="formData.output.encode.bitrate" type="number" class="form-input p-inputtext-sm" placeholder="2000000 (2Mbps)" />
            </div>
          </div>
        </div>

        <div class="form-actions border-t border-surface-200 dark:border-surface-700 pt-4 mt-4">
          <Button label="取消" text severity="secondary" @click="showDialog = false" />
          <Button type="submit" label="保存" :loading="submitting" />
        </div>
      </Form>
    </Dialog>

    <Dialog
      v-model:visible="showPreviewDialog"
      :style="{ width: '72rem', maxWidth: '96vw' }"
      header="Live Preview"
      :modal="true"
      @hide="closePreview"
    >
      <div class="preview-shell">
        <div class="preview-meta" v-if="previewDevice">
          <div>
            <div class="preview-title">{{ previewDevice.name }}</div>
            <div class="preview-subtitle">默认 FLV 直播预览</div>
          </div>
          <code class="preview-url">{{ getPreviewFlvUrl(previewDevice) }}</code>
        </div>

        <div class="preview-stage">
          <video
            ref="previewVideoRef"
            class="preview-video"
            controls
            autoplay
            muted
            playsinline
          ></video>

          <div v-if="previewLoading" class="preview-overlay">
            正在连接直播流...
          </div>

          <div v-if="previewError" class="preview-error">
            {{ previewError }}
          </div>
        </div>
      </div>
    </Dialog>
  </div>
</template>

<style scoped>
.content-section {
  max-width: 100%;
}

.content-card {
  margin-bottom: 1rem;
  border: 1px solid var(--p-content-border-color, #e5e7eb);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
}

.content-card :deep(.p-card-title) {
  font-size: 1.125rem;
}

.content-card :deep(.p-card-subtitle) {
  font-size: 0.8125rem;
}

.content-card :deep(.p-card-content) {
  padding: 0.75rem 0;
}

.content-table {
  margin-top: 0.5rem;
}

.empty-message {
  padding: 2rem;
  text-align: center;
  color: var(--p-text-muted-color);
}

.form-subtitle {
  color: var(--p-text-muted-color);
  display: block;
  margin-bottom: 2rem;
}

.form-container {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.form-row {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 0.5rem;
}

.form-label {
  font-weight: 600;
  width: 6rem;
  flex-shrink: 0;
}

.form-input {
  flex: 1 1 auto;
}

.form-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 1rem;
  padding-top: 1rem;
  border-top: 1px solid var(--p-content-border-color, #e5e7eb);
}

.output-card {
  padding: 1rem;
  border: 1px solid var(--p-content-border-color, #e5e7eb);
  border-radius: 6px;
  margin-bottom: 1rem;
  background-color: var(--p-surface-50, #f9fafb);
}

.output-divider {
  margin-top: 1rem;
  padding-top: 1rem;
  border-top: 1px dashed var(--p-content-border-color, #e5e7eb);
}

.output-divider-title {
  display: block;
  font-size: 0.875rem;
  font-weight: 600;
  margin-bottom: 0.5rem;
  color: var(--p-text-muted-color);
}

.preview-shell {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.preview-meta {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 1rem;
  padding: 0.85rem 1rem;
  border: 1px solid var(--p-content-border-color, #e5e7eb);
  border-radius: 10px;
  background: linear-gradient(135deg, #f7fafc 0%, #eef6ff 100%);
}

.preview-title {
  font-size: 1rem;
  font-weight: 700;
  color: #10233d;
}

.preview-subtitle {
  margin-top: 0.25rem;
  font-size: 0.8125rem;
  color: #5b6b82;
}

.preview-url {
  max-width: 55%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 0.8125rem;
  color: #0f4c81;
}

.preview-stage {
  position: relative;
  overflow: hidden;
  min-height: 30rem;
  border-radius: 16px;
  border: 1px solid #d8e2ef;
  background:
    radial-gradient(circle at top left, rgba(73, 139, 255, 0.16), transparent 32%),
    linear-gradient(160deg, #09111d 0%, #0f1b2c 55%, #08101a 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06);
}

.preview-video {
  display: block;
  width: 100%;
  height: min(70vh, 42rem);
  object-fit: contain;
  background: transparent;
}

.preview-overlay,
.preview-error {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem;
  text-align: center;
  font-size: 0.95rem;
}

.preview-overlay {
  color: #dbe8ff;
  background: rgba(7, 15, 29, 0.48);
  backdrop-filter: blur(4px);
}

.preview-error {
  color: #ffd7d7;
  background: rgba(58, 10, 10, 0.45);
}

@media (max-width: 768px) {
  .preview-meta {
    flex-direction: column;
  }

  .preview-url {
    max-width: 100%;
  }

  .preview-stage {
    min-height: 18rem;
  }

  .preview-video {
    height: min(56vh, 24rem);
  }
}
</style>
