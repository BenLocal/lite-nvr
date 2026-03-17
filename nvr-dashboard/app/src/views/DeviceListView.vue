<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Form from '@primevue/forms/form'
import Button from 'primevue/button'
import Card from 'primevue/card'
import Column from 'primevue/column'
import DataTable from 'primevue/datatable'
import Dialog from 'primevue/dialog'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import Select from 'primevue/select'
import Textarea from 'primevue/textarea'
import { useConfirm } from 'primevue/useconfirm'
import { useToast } from 'primevue/usetoast'
import {
  addDevice,
  listDevices,
  removeDevice,
  updateDevice,
  type DeviceItem,
  type DevicePayload,
} from '../api/device'

type FlvPlayer = {
  attachMediaElement: (element: HTMLVideoElement) => void
  load: () => void
  play: () => Promise<void>
  on?: (event: string, listener: (payload: unknown) => void) => void
  pause?: () => void
  unload?: () => void
  detachMediaElement?: () => void
  destroy: () => void
}

type FlvJs = {
  isSupported: () => boolean
  createPlayer: (mediaDataSource: { type: 'flv'; url: string; isLive: boolean }) => FlvPlayer
}

declare global {
  interface Window {
    flvjs?: FlvJs
  }
}

const toast = useToast()
const confirm = useConfirm()

const loading = ref(false)
const saving = ref(false)
const devices = ref<DeviceItem[]>([])
const dialogVisible = ref(false)
const previewVisible = ref(false)
const editingDevice = ref<DeviceItem | null>(null)
const previewDevice = ref<DeviceItem | null>(null)
const videoRef = ref<HTMLVideoElement | null>(null)
const flvPlayer = ref<FlvPlayer | null>(null)
const previewError = ref('')
const previewMediaInfo = ref<Record<string, unknown>>({})
const previewStats = ref<Record<string, unknown>>({})
const previewInfoVisible = ref(true)
const previewInfoPosition = ref({ x: 20, y: 20 })
const previewInfoDrag = ref<{ startX: number; startY: number; originX: number; originY: number } | null>(null)
const inputTypeOptions = [
  { label: 'RTSP', value: 'rtsp' },
  { label: 'RTMP', value: 'rtmp' },
  { label: '文件', value: 'file' },
  { label: 'V4L2', value: 'v4l2' },
  { label: 'X11 Grab', value: 'x11grab' },
  { label: 'Lavfi', value: 'lavfi' },
]

const initialValues = {
  name: '',
  input_type: 'rtsp',
  input_value: '',
  description: '',
}

onMounted(() => {
  void loadDevices()
})

watch(previewVisible, async (visible) => {
  if (visible) {
    await nextTick()
    await startPreview()
    return
  }
  stopPreview()
})

onBeforeUnmount(() => {
  stopPreview()
})

function resolver({ values }: { values: Record<string, unknown> }) {
  const name = String(values.name ?? '').trim()
  const inputType = String(values.input_type ?? '').trim()
  const inputValue = String(values.input_value ?? '').trim()
  const description = String(values.description ?? '').trim()
  const errors: Record<string, { message: string }[]> = {}

  if (!name) {
    errors.name = [{ message: '请输入设备名称' }]
  }
  if (!inputType) {
    errors.input_type = [{ message: '请输入输入类型' }]
  }
  if (!inputValue) {
    errors.input_value = [{ message: '请输入输入地址或标识' }]
  }

  return {
    values: {
      name,
      input_type: inputType,
      input_value: inputValue,
      description,
    },
    errors,
  }
}

async function loadDevices() {
  loading.value = true
  try {
    devices.value = await listDevices()
  } catch (error) {
    toast.add({
      severity: 'error',
      summary: '加载失败',
      detail: toErrorMessage(error, '设备列表加载失败'),
      life: 2500,
    })
  } finally {
    loading.value = false
  }
}

function openCreateDialog() {
  editingDevice.value = null
  dialogVisible.value = true
}

function openEditDialog(device: DeviceItem) {
  editingDevice.value = device
  dialogVisible.value = true
}

function openPreview(device: DeviceItem) {
  previewDevice.value = device
  previewError.value = ''
  previewMediaInfo.value = {}
  previewStats.value = {}
  previewInfoVisible.value = true
  previewInfoPosition.value = { x: 20, y: 20 }
  previewVisible.value = true
}

function closePreview() {
  previewVisible.value = false
}

async function onSubmit(event: { valid: boolean; values: Record<string, unknown> }) {
  if (!event.valid) {
    return
  }

  const payload: DevicePayload = {
    name: String(event.values.name ?? ''),
    input_type: String(event.values.input_type ?? ''),
    input_value: String(event.values.input_value ?? ''),
    description: String(event.values.description ?? ''),
  }

  saving.value = true
  try {
    if (editingDevice.value) {
      await updateDevice(editingDevice.value.id, payload)
      toast.add({
        severity: 'success',
        summary: '更新成功',
        detail: `设备 ${payload.name} 已更新`,
        life: 2000,
      })
    } else {
      await addDevice(payload)
      toast.add({
        severity: 'success',
        summary: '添加成功',
        detail: `设备 ${payload.name} 已添加`,
        life: 2000,
      })
    }
    dialogVisible.value = false
    await loadDevices()
  } catch (error) {
    toast.add({
      severity: 'error',
      summary: '保存失败',
      detail: toErrorMessage(error, '设备保存失败'),
      life: 2500,
    })
  } finally {
    saving.value = false
  }
}

function confirmDelete(device: DeviceItem) {
  confirm.require({
    header: '删除设备',
    message: `确认删除设备“${device.name}”吗？`,
    icon: 'pi pi-exclamation-triangle',
    rejectLabel: '取消',
    acceptLabel: '删除',
    acceptClass: 'p-button-danger',
    accept: async () => {
      try {
        await removeDevice(device.id)
        toast.add({
          severity: 'success',
          summary: '删除成功',
          detail: `设备 ${device.name} 已删除`,
          life: 2000,
        })
        await loadDevices()
      } catch (error) {
        toast.add({
          severity: 'error',
          summary: '删除失败',
          detail: toErrorMessage(error, '设备删除失败'),
          life: 2500,
        })
      }
    },
  })
}

async function startPreview() {
  stopPreview()
  previewError.value = ''
  const video = videoRef.value
  const flvUrl = previewDevice.value?.flv_url || (previewDevice.value ? buildFlvUrl(previewDevice.value.id) : '')

  if (!video || !flvUrl) {
    return
  }

  try {
    const flvjs = await ensureFlvJs()
    if (!flvjs?.isSupported()) {
      video.src = flvUrl
      await video.play()
      return
    }

    const player = flvjs.createPlayer({
      type: 'flv',
      url: flvUrl,
      isLive: true,
    })
    player.on?.('media_info', (payload) => {
      previewMediaInfo.value = {
        ...previewMediaInfo.value,
        ...(isRecord(payload) ? payload : {}),
      }
    })
    player.on?.('statistics_info', (payload) => {
      previewStats.value = {
        ...previewStats.value,
        ...(isRecord(payload) ? payload : {}),
      }
    })
    video.addEventListener('loadedmetadata', syncVideoMetadata, { once: true })
    flvPlayer.value = player
    player.attachMediaElement(video)
    player.load()
    await player.play()
  } catch (error) {
    previewError.value = toErrorMessage(error, 'FLV 预览启动失败')
  }
}

function stopPreview() {
  const video = videoRef.value
  if (flvPlayer.value) {
    flvPlayer.value.pause?.()
    flvPlayer.value.unload?.()
    flvPlayer.value.detachMediaElement?.()
    flvPlayer.value.destroy()
    flvPlayer.value = null
  }

  if (video) {
    video.pause()
    video.removeAttribute('src')
    video.load()
  }

  previewMediaInfo.value = {}
  previewStats.value = {}
  previewInfoDrag.value = null
}

async function ensureFlvJs() {
  if (window.flvjs) {
    return window.flvjs
  }

  await new Promise<void>((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>('script[data-flvjs="true"]')
    if (existing) {
      existing.addEventListener('load', () => resolve(), { once: true })
      existing.addEventListener('error', () => reject(new Error('flv.js load failed')), {
        once: true,
      })
      return
    }

    const script = document.createElement('script')
    script.src = 'https://cdn.jsdelivr.net/npm/flv.js@1.6.2/dist/flv.min.js'
    script.async = true
    script.dataset.flvjs = 'true'
    script.onload = () => resolve()
    script.onerror = () => reject(new Error('flv.js load failed'))
    document.head.appendChild(script)
  })

  return window.flvjs
}

function toErrorMessage(error: unknown, fallback: string) {
  if (error instanceof Error && error.message) {
    return error.message
  }
  return fallback
}

function formatTime(value: string) {
  return new Date(value).toLocaleString('zh-CN', { hour12: false })
}

function syncVideoMetadata() {
  const video = videoRef.value
  if (!video) {
    return
  }
  previewMediaInfo.value = {
    ...previewMediaInfo.value,
    width: Number(video.videoWidth) || previewMediaInfo.value.width,
    height: Number(video.videoHeight) || previewMediaInfo.value.height,
  }
}

function buildFlvUrl(deviceId: string) {
  return `http://127.0.0.1:8553/live/${encodeURIComponent(deviceId)}.live.flv`
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function formatInfoValue(value: unknown, suffix = '') {
  if (value === null || value === undefined || value === '') {
    return '-'
  }
  return `${value}${suffix}`
}

function previewInfoItems() {
  const media = previewMediaInfo.value
  const stats = previewStats.value
  return [
    { label: '视频编码', value: formatInfoValue(media.videoCodec) },
    { label: '音频编码', value: formatInfoValue(media.audioCodec) },
    { label: '分辨率', value: media.width && media.height ? `${media.width} x ${media.height}` : '-' },
    { label: 'FPS', value: formatInfoValue(media.fps ?? stats.fps) },
    { label: '码率', value: formatInfoValue(stats.speed, ' KB/s') },
    { label: '音频采样率', value: formatInfoValue(media.audioSampleRate, ' Hz') },
    { label: '声道数', value: formatInfoValue(media.audioChannelCount) },
    { label: '丢帧', value: formatInfoValue(stats.droppedFrames) },
  ]
}

function openPreviewInfo() {
  previewInfoVisible.value = true
}

function closePreviewInfo() {
  previewInfoVisible.value = false
}

function startPreviewInfoDrag(event: PointerEvent) {
  if ((event.target as HTMLElement | null)?.closest('.preview-info-close')) {
    return
  }
  previewInfoDrag.value = {
    startX: event.clientX,
    startY: event.clientY,
    originX: previewInfoPosition.value.x,
    originY: previewInfoPosition.value.y,
  }
  window.addEventListener('pointermove', onPreviewInfoDrag)
  window.addEventListener('pointerup', stopPreviewInfoDrag, { once: true })
}

function onPreviewInfoDrag(event: PointerEvent) {
  if (!previewInfoDrag.value) {
    return
  }
  const nextX = previewInfoDrag.value.originX + event.clientX - previewInfoDrag.value.startX
  const nextY = previewInfoDrag.value.originY + event.clientY - previewInfoDrag.value.startY
  previewInfoPosition.value = {
    x: Math.max(8, nextX),
    y: Math.max(8, nextY),
  }
}

function stopPreviewInfoDrag() {
  previewInfoDrag.value = null
  window.removeEventListener('pointermove', onPreviewInfoDrag)
}

async function copyText(value: string, label: string) {
  try {
    await navigator.clipboard.writeText(value)
    toast.add({
      severity: 'success',
      summary: '复制成功',
      detail: `${label}已复制到剪贴板`,
      life: 1800,
    })
  } catch (error) {
    toast.add({
      severity: 'error',
      summary: '复制失败',
      detail: toErrorMessage(error, `${label}复制失败`),
      life: 2200,
    })
  }
}
</script>

<template>
  <div class="content-section">
    <Card class="content-card">
      <template #title>
        <div class="page-header">
          <div>
            <div>设备管理</div>
            <div class="page-subtitle">从后台接口读取设备列表，并支持增删改查与实时预览。</div>
          </div>
          <div class="page-actions">
            <Button icon="pi pi-refresh" text aria-label="刷新" @click="loadDevices" />
            <Button icon="pi pi-plus" label="添加设备" @click="openCreateDialog" />
          </div>
        </div>
      </template>
      <template #content>
        <DataTable
          :value="devices"
          :loading="loading"
          striped-rows
          scrollable
          class="content-table"
          responsive-layout="scroll"
        >
          <Column field="name" header="设备名称" :style="{ width: '12rem', minWidth: '12rem', maxWidth: '12rem' }">
            <template #body="{ data }">
              <span class="single-line-text" :title="data.name">{{ data.name }}</span>
            </template>
          </Column>
          <Column field="id" header="设备 ID" :style="{ width: '17rem' }">
            <template #body="{ data }">
              <div class="copy-cell copy-cell-id" :title="data.id">
                <span class="mono-text ellipsis-text">{{ data.id }}</span>
                <Button
                  icon="pi pi-copy"
                  text
                  rounded
                  class="copy-button"
                  aria-label="复制设备 ID"
                  @click="copyText(data.id, '设备 ID')"
                />
              </div>
            </template>
          </Column>
          <Column field="input_type" header="输入类型" :style="{ width: '8rem', minWidth: '8rem', maxWidth: '8rem' }">
            <template #body="{ data }">
              <span class="single-line-text" :title="data.input_type">{{ data.input_type }}</span>
            </template>
          </Column>
          <Column field="input_value" header="输入地址/标识" :style="{ width: '24rem' }">
            <template #body="{ data }">
              <div class="copy-cell copy-cell-input" :title="data.input_value">
                <span class="mono-text ellipsis-text">{{ data.input_value }}</span>
                <Button
                  icon="pi pi-copy"
                  text
                  rounded
                  class="copy-button"
                  aria-label="复制输入地址"
                  @click="copyText(data.input_value, '输入地址')"
                />
              </div>
            </template>
          </Column>
          <Column field="updated_at" header="更新时间">
            <template #body="{ data }">
              {{ formatTime(data.updated_at) }}
            </template>
          </Column>
          <Column
            header="操作"
            :exportable="false"
            class="action-column"
            frozen
            align-frozen="right"
            :style="{ width: '9rem', minWidth: '9rem', maxWidth: '9rem' }"
          >
            <template #body="{ data }">
              <div class="row-actions">
                <Button
                  icon="pi pi-play"
                  text
                  rounded
                  severity="success"
                  aria-label="预览"
                  @click="openPreview(data)"
                />
                <Button
                  icon="pi pi-pencil"
                  text
                  rounded
                  aria-label="编辑"
                  @click="openEditDialog(data)"
                />
                <Button
                  icon="pi pi-trash"
                  text
                  rounded
                  severity="danger"
                  aria-label="删除"
                  @click="confirmDelete(data)"
                />
              </div>
            </template>
          </Column>
          <template #empty>
            <div class="empty-message">暂无设备数据，点击右上角“添加设备”开始接入。</div>
          </template>
        </DataTable>
      </template>
    </Card>

    <Dialog
      v-model:visible="dialogVisible"
      modal
      :header="editingDevice ? '编辑设备' : '添加设备'"
      :style="{ width: 'min(40rem, calc(100vw - 2rem))' }"
    >
      <Form
        v-slot="$form"
        :resolver="resolver"
        :initial-values="editingDevice ?? initialValues"
        class="device-form"
        @submit="onSubmit"
      >
        <div class="field-grid">
          <div class="field">
            <label for="name">设备名称</label>
            <InputText id="name" name="name" class="field-input" :invalid="$form.name?.invalid" />
            <Message v-if="$form.name?.invalid" severity="error" size="small" variant="simple">
              {{ $form.name.error?.message }}
            </Message>
          </div>
          <div class="field">
            <label for="input_type">输入类型</label>
            <Select
              id="input_type"
              name="input_type"
              :options="inputTypeOptions"
              option-label="label"
              option-value="value"
              class="field-input"
              placeholder="请选择输入类型"
              :invalid="$form.input_type?.invalid"
            />
            <Message v-if="$form.input_type?.invalid" severity="error" size="small" variant="simple">
              {{ $form.input_type.error?.message }}
            </Message>
          </div>
        </div>

        <div class="field">
          <label for="input_value">输入地址/标识</label>
          <InputText
            id="input_value"
            name="input_value"
            class="field-input"
            placeholder="如 rtsp://camera/live"
            :invalid="$form.input_value?.invalid"
          />
          <Message v-if="$form.input_value?.invalid" severity="error" size="small" variant="simple">
            {{ $form.input_value.error?.message }}
          </Message>
        </div>

        <div class="field">
          <label for="description">备注</label>
          <Textarea id="description" name="description" class="field-input" rows="3" />
        </div>

        <div class="dialog-actions">
          <Button type="button" label="取消" text @click="dialogVisible = false" />
          <Button type="submit" :label="editingDevice ? '保存修改' : '确认添加'" :loading="saving" />
        </div>
      </Form>
    </Dialog>

    <Dialog
      v-model:visible="previewVisible"
      modal
      header="实时预览"
      :style="{ width: 'min(60rem, calc(100vw - 2rem))' }"
      :content-style="{ overflow: 'hidden' }"
      @hide="closePreview"
    >
      <div class="preview-shell">
        <Button
          icon="pi pi-info-circle"
          text
          rounded
          class="preview-info-toggle"
          aria-label="打开流信息"
          @click="openPreviewInfo"
        />
        <div class="preview-meta">
          <div class="preview-title">{{ previewDevice?.name }}</div>
          <div class="preview-url">{{ previewDevice?.flv_url || (previewDevice ? buildFlvUrl(previewDevice.id) : '') }}</div>
        </div>
        <Message v-if="previewError" severity="error" :closable="false">{{ previewError }}</Message>
        <div class="preview-stage">
          <div
            v-if="previewInfoVisible"
            class="preview-info-panel"
            :style="{ transform: `translate(${previewInfoPosition.x}px, ${previewInfoPosition.y}px)` }"
          >
            <div class="preview-info-panel-header" @pointerdown="startPreviewInfoDrag">
              <div class="preview-info-panel-title">流信息</div>
              <Button
                icon="pi pi-times"
                text
                rounded
                class="preview-info-close"
                aria-label="关闭流信息"
                @click="closePreviewInfo"
              />
            </div>
            <div class="preview-info-grid">
              <div v-for="item in previewInfoItems()" :key="item.label" class="preview-info-card">
                <div class="preview-info-label">{{ item.label }}</div>
                <div class="preview-info-value">{{ item.value }}</div>
              </div>
            </div>
          </div>
          <video
            ref="videoRef"
            class="preview-video"
            controls
            autoplay
            muted
            playsinline
          />
        </div>
      </div>
    </Dialog>
  </div>
</template>

<style scoped>
.page-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
}

.page-subtitle {
  margin-top: 0.25rem;
  font-size: 0.875rem;
  color: var(--p-text-muted-color);
}

.page-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.content-table {
  margin-top: 0.5rem;
}

.empty-message {
  padding: 2rem;
  text-align: center;
  color: var(--p-text-muted-color);
}

.mono-text {
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
  font-size: 0.8125rem;
}

.single-line-text {
  display: inline-block;
  max-width: 100%;
  white-space: nowrap;
}

.copy-cell {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  min-width: 0;
  width: 100%;
}

.copy-cell-id {
  max-width: 15rem;
}

.copy-cell-input {
  max-width: 22rem;
}

.ellipsis-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.copy-button {
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.16s ease;
}

.copy-cell:hover .copy-button,
.copy-cell:focus-within .copy-button {
  opacity: 1;
  pointer-events: auto;
}

.row-actions {
  display: flex;
  align-items: center;
  gap: 0.125rem;
}

.device-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.field-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.field label {
  font-size: 0.875rem;
  font-weight: 600;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
}

.preview-shell {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  overflow: hidden;
  position: relative;
}

.preview-info-toggle {
  position: absolute;
  top: 0.25rem;
  right: 0.25rem;
  z-index: 3;
  background: rgb(8 13 22 / 72%);
  color: white;
  backdrop-filter: blur(10px);
}

.preview-stage {
  position: relative;
  overflow: hidden;
}

.preview-info-panel {
  position: absolute;
  left: 0;
  top: 0;
  z-index: 2;
  width: min(32rem, calc(100% - 1rem));
  max-width: calc(100% - 1rem);
  padding: 0.75rem;
  border: 1px solid rgb(255 255 255 / 12%);
  border-radius: 0.9rem;
  background: rgb(8 13 22 / 78%);
  box-shadow: 0 14px 40px rgb(0 0 0 / 26%);
  backdrop-filter: blur(14px);
}

.preview-info-panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.75rem;
  cursor: move;
  user-select: none;
}

.preview-info-panel-title {
  font-size: 0.9rem;
  font-weight: 700;
  color: rgb(255 255 255 / 92%);
}

.preview-info-close {
  color: white;
}

.preview-info-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.6rem;
}

.preview-info-card {
  min-width: 0;
  padding: 0.65rem 0.75rem;
  border: 1px solid rgb(255 255 255 / 10%);
  border-radius: 0.75rem;
  background: rgb(255 255 255 / 6%);
}

.preview-info-label {
  font-size: 0.75rem;
  color: rgb(255 255 255 / 58%);
}

.preview-info-value {
  margin-top: 0.35rem;
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
  font-size: 0.9rem;
  font-weight: 600;
  color: rgb(255 255 255 / 92%);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.preview-meta {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.preview-title {
  font-size: 1rem;
  font-weight: 700;
}

.preview-url {
  font-size: 0.875rem;
  color: var(--p-text-muted-color);
  word-break: break-all;
}

.preview-video {
  width: 100%;
  height: min(70vh, 36rem);
  min-height: 18rem;
  max-height: 70vh;
  border-radius: 0.75rem;
  object-fit: contain;
  background:
    radial-gradient(circle at top, rgb(41 98 255 / 15%), transparent 55%),
    linear-gradient(180deg, rgb(10 17 29), rgb(20 26 36));
}

@media (width <= 768px) {
  .page-header,
  .page-actions,
  .field-grid {
    display: flex;
    flex-direction: column;
  }

  .page-actions {
    align-items: stretch;
  }

  .preview-info-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .preview-info-panel {
    width: calc(100% - 1rem);
  }

  .preview-video {
    height: min(52vh, 20rem);
    min-height: 14rem;
  }
}
</style>
