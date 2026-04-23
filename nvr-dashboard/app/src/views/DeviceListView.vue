<script setup lang="ts">
import { onMounted, ref } from "vue";
import Form from "@primevue/forms/form";
import Button from "primevue/button";
import Card from "primevue/card";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Message from "primevue/message";
import Select from "primevue/select";
import Textarea from "primevue/textarea";
import { useConfirm } from "primevue/useconfirm";
import FlvPreviewPlayer from "../components/FlvPreviewPlayer.vue";
import {
  addDevice,
  listDevices,
  removeDevice,
  updateDevice,
  type DeviceItem,
  type DevicePayload,
} from "../api/device";
import { useAppToast } from "../utils/toast";

const appToast = useAppToast();
const confirm = useConfirm();

const loading = ref(false);
const saving = ref(false);
const devices = ref<DeviceItem[]>([]);
const dialogVisible = ref(false);
const previewVisible = ref(false);
const editingDevice = ref<DeviceItem | null>(null);
const previewDevice = ref<DeviceItem | null>(null);
const inputTypeOptions = [
  { label: "RTSP", value: "rtsp" },
  { label: "RTMP", value: "rtmp" },
  { label: "文件", value: "file" },
  { label: "V4L2", value: "v4l2" },
  { label: "X11 Grab", value: "x11grab" },
  { label: "Lavfi", value: "lavfi" },
];

const initialValues = {
  name: "",
  input_type: "rtsp",
  input_value: "",
  description: "",
};

onMounted(() => {
  void loadDevices();
});

function resolver({ values }: { values: Record<string, unknown> }) {
  const name = String(values.name ?? "").trim();
  const inputType = String(values.input_type ?? "").trim();
  const inputValue = String(values.input_value ?? "").trim();
  const description = String(values.description ?? "").trim();
  const errors: Record<string, { message: string }[]> = {};

  if (!name) {
    errors.name = [{ message: "请输入设备名称" }];
  }
  if (!inputType) {
    errors.input_type = [{ message: "请输入输入类型" }];
  }
  if (!inputValue) {
    errors.input_value = [{ message: "请输入输入地址或标识" }];
  }

  return {
    values: {
      name,
      input_type: inputType,
      input_value: inputValue,
      description,
    },
    errors,
  };
}

async function loadDevices() {
  loading.value = true;
  try {
    devices.value = await listDevices();
  } catch (error) {
    appToast.errorFrom("加载失败", error, "设备列表加载失败");
  } finally {
    loading.value = false;
  }
}

function openCreateDialog() {
  editingDevice.value = null;
  dialogVisible.value = true;
}

function openEditDialog(device: DeviceItem) {
  editingDevice.value = device;
  dialogVisible.value = true;
}

function openPreview(device: DeviceItem) {
  previewDevice.value = device;
  previewVisible.value = true;
}

function closePreview() {
  previewVisible.value = false;
}

async function onSubmit(event: { valid: boolean; values: Record<string, unknown> }) {
  if (!event.valid) {
    return;
  }

  const payload: DevicePayload = {
    name: String(event.values.name ?? ""),
    input_type: String(event.values.input_type ?? ""),
    input_value: String(event.values.input_value ?? ""),
    description: String(event.values.description ?? ""),
  };

  saving.value = true;
  try {
    if (editingDevice.value) {
      await updateDevice(editingDevice.value.id, payload);
      appToast.success("更新成功", `设备 ${payload.name} 已更新`);
    } else {
      await addDevice(payload);
      appToast.success("添加成功", `设备 ${payload.name} 已添加`);
    }
    dialogVisible.value = false;
    await loadDevices();
  } catch (error) {
    appToast.errorFrom("保存失败", error, "设备保存失败");
  } finally {
    saving.value = false;
  }
}

function confirmDelete(device: DeviceItem) {
  confirm.require({
    header: "删除设备",
    message: `确认删除设备“${device.name}"吗？`,
    icon: "pi pi-exclamation-triangle",
    rejectLabel: "取消",
    acceptLabel: "删除",
    acceptClass: "p-button-danger",
    accept: async () => {
      try {
        await removeDevice(device.id);
        appToast.success("删除成功", `设备 ${device.name} 已删除`);
        await loadDevices();
      } catch (error) {
        appToast.errorFrom("删除失败", error, "设备删除失败");
      }
    },
  });
}

function formatTime(value: string) {
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

function buildFlvUrl(deviceId: string) {
  return `http://127.0.0.1:8553/live/${encodeURIComponent(deviceId)}.live.flv`;
}

async function copyText(value: string, label: string) {
  try {
    await navigator.clipboard.writeText(value);
    appToast.success("复制成功", `${label}已复制到剪贴板`, 1800);
  } catch (error) {
    appToast.errorFrom("复制失败", error, `${label}复制失败`, 2200);
  }
}
</script>

<template>
  <div class="content-section device-page">
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">设备管理</h1>
        <p class="page-subtitle">实时监控和管理接入的 NVR 设备</p>
      </div>
      <div class="page-actions">
        <Button icon="pi pi-refresh" text aria-label="刷新" @click="loadDevices" />
        <Button icon="pi pi-plus" label="添加设备" @click="openCreateDialog" />
      </div>
    </div>

    <div v-if="!loading && !devices.length" class="empty-state device-empty-state">
      <i class="pi pi-video empty-state-icon" />
      <p class="empty-state-text">暂无设备数据</p>
      <p class="device-empty-state-hint">点击右上角“添加设备”开始接入</p>
    </div>

    <Card v-else class="data-card">
      <template #content>
        <DataTable
          :value="devices"
          :loading="loading"
          striped-rows
          scrollable
          scroll-height="flex"
          class="content-table"
          responsive-layout="scroll"
        >
          <Column
            field="name"
            header="设备名称"
            style="width: 12rem; min-width: 12rem; max-width: 12rem"
          >
            <template #body="{ data }">
              <span class="single-line-text" :title="data.name">{{ data.name }}</span>
            </template>
          </Column>
          <Column field="id" header="设备 ID" style="width: 17rem; min-width: 17rem">
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
          <Column
            field="input_type"
            header="输入类型"
            style="width: 8rem; min-width: 8rem; max-width: 8rem"
          >
            <template #body="{ data }">
              <span class="single-line-text" :title="data.input_type">{{ data.input_type }}</span>
            </template>
          </Column>
          <Column field="input_value" header="输入地址/标识" style="width: 24rem; min-width: 24rem">
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
          <Column field="updated_at" header="更新时间" style="width: 12rem; min-width: 12rem">
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
            style="width: 9rem; min-width: 9rem; max-width: 9rem"
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
        </DataTable>
      </template>
    </Card>

    <Dialog
      v-model:visible="dialogVisible"
      modal
      :header="editingDevice ? '编辑设备' : '添加设备'"
      class="device-dialog"
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
              size="small"
              class="field-input"
              placeholder="请选择输入类型"
              :invalid="$form.input_type?.invalid"
            />
            <Message
              v-if="$form.input_type?.invalid"
              severity="error"
              size="small"
              variant="simple"
            >
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
          <Button
            type="submit"
            :label="editingDevice ? '保存修改' : '确认添加'"
            :loading="saving"
          />
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
        <div class="preview-meta">
          <div class="preview-title">{{ previewDevice?.name }}</div>
          <div class="preview-url">
            {{ previewDevice?.flv_url || (previewDevice ? buildFlvUrl(previewDevice.id) : "") }}
          </div>
        </div>
        <FlvPreviewPlayer
          :url="previewDevice?.flv_url || (previewDevice ? buildFlvUrl(previewDevice.id) : '')"
        />
      </div>
    </Dialog>
  </div>
</template>

<style scoped>
/* Page-specific styles - matching DashboardView style */

.device-page {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.device-page .page-header {
  flex: 0 0 auto;
}

.data-card {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: slide-up 0.5s ease-out 0.1s backwards;
}

:deep(.data-card .p-card-body) {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

:deep(.data-card .p-card-content) {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 0;
}

:deep(.content-table) {
  flex: 1 1 auto;
  min-height: 0;
  background: transparent;
}

:deep(.content-table .p-datatable-table-container) {
  border-radius: 0.75rem;
}

.device-empty-state {
  flex: 1 1 auto;
  min-height: 14rem;
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

:deep(.device-dialog .field) {
  padding: 0.875rem;
  background: rgb(15 23 42 / 42%);
  border: 1px solid rgb(148 163 184 / 10%);
  border-radius: 0.75rem;
}

:deep(.device-dialog .field label) {
  color: #cbd5e1;
  font-size: 0.75rem;
  font-weight: 600;
  letter-spacing: 0.02em;
}

:deep(.device-dialog .p-message) {
  margin-top: 0.125rem;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 0.25rem;
  padding-top: 1rem;
  border-top: 1px solid rgb(148 163 184 / 10%);
}

.preview-shell {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  overflow: hidden;
}

.preview-meta {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.preview-title {
  font-size: 0.9375rem;
  font-weight: 600;
  color: #e2e8f0;
}

.preview-url {
  font-size: 0.75rem;
  color: #64748b;
  word-break: break-all;
  font-family: SFMono-Regular, Consolas, "Liberation Mono", monospace;
}

.device-empty-state-hint {
  margin: 0;
  font-size: 0.8125rem;
  color: #94a3b8;
}
</style>
