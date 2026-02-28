<script setup lang="ts">
import { ref, onMounted } from 'vue'
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

interface Device {
  id: number
  name: string
  status: string
  input: any
  outputs: any[]
  created_at?: string
  updated_at?: string
}

const devices = ref<Device[]>([])
const loading = ref(true) // Changed default loading to true
const showDialog = ref(false)
const isEdit = ref(false)
const submitting = ref(false)
const formData = ref({
  id: 0,
  name: '',
  input: {
    t: 'net',
    i: ''
  },
  outputs: [] as any[]
})

const inputOptions = [
  { label: 'Network 流 (Rtsp/Rtmp/HTTP)', value: 'net' },
  { label: '本地设备 (v4l2/DirectShow)', value: 'v4l2' },
  { label: '屏幕录制 (x11grab/gdigrab)', value: 'x11grab' }
]

const outputOptions = [
  { label: 'Network 推流 (Rtsp/Rtmp)', value: 'net' },
  { label: 'ZLM 内部转发 (推荐)', value: 'zlm' },
  { label: 'RawFrame (原始视频帧)', value: 'raw_frame' },
  { label: 'RawPacket (原始数据包)', value: 'raw_packet' }
]

const addOutput = () => {
  formData.value.outputs.push({
    t: 'zlm',
    zlm: { app: 'live', stream: formData.value.name || 'stream' },
    net: { url: '', format: 'rtsp' },
    encode: { preset: '', bitrate: null }
  })
}

const removeOutput = (index: number) => {
  formData.value.outputs.splice(index, 1)
}

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
    outputs: []
  }
}

const openAddDevice = () => {
  isEdit.value = false
  resetForm()
  showDialog.value = true
}

const openEditDevice = (device: Device) => {
  isEdit.value = true
  // Deep copy the input and outputs so modifications don't instantly reflect
  formData.value = {
    ...device,
    input: device.input ? JSON.parse(JSON.stringify(device.input)) : { t: 'net', i: '' },
    outputs: device.outputs ? JSON.parse(JSON.stringify(device.outputs)) : []
  }
  showDialog.value = true
}

const saveDevice = async () => {
  if (!formData.value.name || !formData.value.input.i) return // Updated validation

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
        input: formData.value.input, // Added input
        outputs: formData.value.outputs // Added outputs
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
          <Column header="操作" :exportable="false" style="min-width: 12rem">
            <template #body="slotProps">
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

        <!-- Outputs Section -->
        <div class="flex justify-between items-center mb-2">
          <h3 class="text-lg font-bold mb-0">输出流配置 (Outputs)</h3>
          <Button label="添加输出" icon="pi pi-plus" size="small" @click="addOutput" />
        </div>

        <div v-for="(out, index) in formData.outputs" :key="index" class="output-card">
          <Button icon="pi pi-times" text rounded severity="danger" class="btn-remove" @click="removeOutput(index)" />
          
          <div class="form-row">
            <label class="form-label">输出类型</label>
            <Select v-model="out.t" :options="outputOptions" optionLabel="label" optionValue="value" class="form-input" />
          </div>

          <!-- ZLM Config -->
          <div v-if="out.t === 'zlm'">
            <div class="form-row">
              <label class="form-label">ZLM App</label>
              <InputText v-model="out.zlm.app" class="form-input" placeholder="live" />
            </div>
            <div class="form-row">
              <label class="form-label">ZLM Stream</label>
              <InputText v-model="out.zlm.stream" class="form-input" placeholder="stream" />
            </div>
          </div>

          <!-- Net Config -->
          <div v-if="out.t === 'net'">
            <div class="form-row">
              <label class="form-label">推流地址</label>
              <InputText v-model="out.net.url" class="form-input" placeholder="rtmp://server/live/stream" />
            </div>
            <div class="form-row">
              <label class="form-label">格式</label>
              <InputText v-model="out.net.format" class="form-input" placeholder="flv" />
            </div>
          </div>

          <!-- Encode Config (Optional) -->
          <div class="output-divider">
            <span class="output-divider-title">高级编码选项 (留空则使用默认配置)</span>
            <div class="form-row">
              <label class="form-label">x264 Preset</label>
              <InputText v-model="out.encode.preset" class="form-input p-inputtext-sm" placeholder="ultrafast, superfast..." />
            </div>
            <div class="form-row">
              <label class="form-label">目标码率(bps)</label>
              <InputText v-model="out.encode.bitrate" type="number" class="form-input p-inputtext-sm" placeholder="2000000 (2Mbps)" />
            </div>
          </div>
        </div>

        <div v-if="formData.outputs.length === 0" class="text-center p-4 text-surface-500">
          目前没有添加任何输出流。设备接收到数据后将被直接丢弃。
        </div>

        <div class="form-actions border-t border-surface-200 dark:border-surface-700 pt-4 mt-4">
          <Button label="取消" text severity="secondary" @click="showDialog = false" />
          <Button type="submit" label="保存" :loading="submitting" />
        </div>
      </Form>
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
  position: relative;
}

.btn-remove {
  position: absolute;
  top: 0.5rem;
  right: 0.5rem;
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
</style>
