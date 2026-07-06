<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import Form from "@primevue/forms/form";
import Button from "primevue/button";
import Card from "primevue/card";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import InputNumber from "primevue/inputnumber";
import InputText from "primevue/inputtext";
import Message from "primevue/message";
import Password from "primevue/password";
import Select from "primevue/select";
import Slider from "primevue/slider";
import Tag from "primevue/tag";
import Textarea from "primevue/textarea";
import ToggleSwitch from "primevue/toggleswitch";
import { useConfirm } from "primevue/useconfirm";
import FlvPreviewPlayer from "../components/FlvPreviewPlayer.vue";
import TranscriptPanel from "../components/TranscriptPanel.vue";
import {
  addDevice,
  listDevices,
  removeDevice,
  updateDevice,
  type DeviceItem,
  type DevicePayload,
} from "../api/device";
import {
  getGbCatalog,
  getGbDevices,
  getGbStreams,
  ptzControl,
  type GbChannel,
  type GbDevice,
  type GbStream,
} from "../api/gb";
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

// GB28181 pickers use standalone refs (not @primevue/forms fields) because their
// options are loaded on demand from the live registrar/catalog API.
const gbDevices = ref<GbDevice[]>([]);
const gbChannels = ref<GbChannel[]>([]);
const gbDeviceId = ref<string>("");
const gbChannelId = ref<string>("");

async function loadGbDevices() {
  try {
    gbDevices.value = await getGbDevices();
  } catch {
    gbDevices.value = [];
  }
}

// Live-status polling for gb28181 device rows: maps stream_id (== device id)
// to its current ZLM publishing status, refreshed every 5s while this view is mounted.
const gbStreamStatus = ref<Record<string, GbStream>>({});
async function loadGbStreams() {
  try {
    const list = await getGbStreams();
    const map: Record<string, GbStream> = {};
    for (const s of list) map[s.stream_id] = s;
    gbStreamStatus.value = map;
  } catch {
    // Keep the last-known status on a transient poll failure instead of
    // flashing every row to 空闲; the next successful poll reconciles.
  }
}
let gbTimer: ReturnType<typeof setInterval> | undefined;

async function onGbDeviceChange(deviceId: string) {
  gbDeviceId.value = deviceId;
  gbChannelId.value = "";
  gbChannels.value = [];
  if (!deviceId) return;
  try {
    const channels = await getGbCatalog(deviceId);
    // Guard against a stale response: if the user switched devices while this
    // catalog was loading, drop it so we don't show device A's channels under B.
    if (gbDeviceId.value !== deviceId) return;
    gbChannels.value = channels;
  } catch {
    if (gbDeviceId.value !== deviceId) return;
    gbChannels.value = [];
  }
}

function resetGbFields() {
  gbDeviceId.value = "";
  gbChannelId.value = "";
  gbChannels.value = [];
}

// PTZ (云台) control for gb28181 devices. The target is resolved from the
// device's input_value JSON ({device_id, channel_id}); moves are press-and-hold
// (move on press, stop on release/leave) and presets are one-shot clicks.
const ptzDialogVisible = ref(false);
const ptzTarget = ref<{ device_id: string; channel_id: string } | null>(null);
const ptzSpeed = ref(128);
const ptzPreset = ref(1);

function openPtzDialog(device: DeviceItem) {
  try {
    const cfg = JSON.parse(device.input_value) as {
      device_id?: string;
      channel_id?: string;
    };
    ptzTarget.value = {
      device_id: cfg.device_id ?? "",
      channel_id: cfg.channel_id ?? "",
    };
    ptzDialogVisible.value = true;
  } catch {
    ptzTarget.value = null;
    appToast.errorFrom("云台", null, "无法解析国标设备通道");
  }
}

async function sendPtz(command: string, preset?: number) {
  if (!ptzTarget.value) return;
  try {
    await ptzControl({
      device_id: ptzTarget.value.device_id,
      channel_id: ptzTarget.value.channel_id,
      command,
      speed: ptzSpeed.value,
      preset,
    });
  } catch {
    // Best-effort: a failed PTZ command shouldn't disrupt the UI. A dropped
    // move still gets a stop on release, so the camera won't run away.
  }
}

// Press-and-hold: send the move on press, stop on release/leave/cancel.
function ptzPress(command: string) {
  void sendPtz(command);
}

function ptzRelease() {
  void sendPtz("stop");
}

const inputTypeOptions = [
  { label: "RTSP", value: "rtsp" },
  { label: "RTMP", value: "rtmp" },
  { label: "文件", value: "file" },
  { label: "V4L2", value: "v4l2" },
  { label: "X11 Grab", value: "x11grab" },
  { label: "Lavfi", value: "lavfi" },
  { label: "小米摄像头", value: "xiaomi" },
  { label: "国标 GB28181", value: "gb28181" },
];

// go2rtc Xiaomi cloud regions ("" = mainland China).
const regionOptions = [
  { label: "中国大陆", value: "" },
  { label: "德国 (de)", value: "de" },
  { label: "印度 (i2)", value: "i2" },
  { label: "俄罗斯 (ru)", value: "ru" },
  { label: "新加坡 (sg)", value: "sg" },
  { label: "美国 (us)", value: "us" },
];

const initialValues = {
  name: "",
  input_type: "rtsp",
  input_value: "",
  description: "",
  include_audio: false,
  record: true,
  // Xiaomi-only structured fields; serialized into input_value on submit.
  xm_user_id: "",
  xm_token: "",
  xm_region: "",
  xm_did: "",
  xm_model: "",
  xm_ip: "",
};

// Split a xiaomi device's input_value JSON back into the xm_* form fields when
// editing; other input types keep the single input_value field.
const formInitialValues = computed(() => {
  const device = editingDevice.value;
  if (!device) {
    return initialValues;
  }
  const base = {
    ...initialValues,
    name: device.name,
    input_type: device.input_type,
    input_value: device.input_value,
    description: device.description,
    include_audio: device.include_audio,
    record: device.record,
  };
  if (device.input_type === "xiaomi" && device.input_value) {
    try {
      const cfg = JSON.parse(device.input_value) as Partial<
        Record<"user_id" | "token" | "region" | "did" | "model" | "ip", string>
      >;
      base.xm_user_id = cfg.user_id ?? "";
      base.xm_token = cfg.token ?? "";
      base.xm_region = cfg.region ?? "";
      base.xm_did = cfg.did ?? "";
      base.xm_model = cfg.model ?? "";
      base.xm_ip = cfg.ip ?? "";
    } catch {
      // malformed config — leave the xm_* fields blank
    }
  }
  return base;
});

onMounted(() => {
  void loadDevices();
  loadGbStreams();
  gbTimer = setInterval(loadGbStreams, 5000);
});

onUnmounted(() => {
  if (gbTimer) clearInterval(gbTimer);
});

function resolver({ values }: { values: Record<string, unknown> }) {
  const name = String(values.name ?? "").trim();
  const inputType = String(values.input_type ?? "").trim();
  const description = String(values.description ?? "").trim();
  const errors: Record<string, { message: string }[]> = {};

  if (!name) {
    errors.name = [{ message: "请输入设备名称" }];
  }
  if (!inputType) {
    errors.input_type = [{ message: "请选择输入类型" }];
  }

  const cleaned: Record<string, unknown> = {
    name,
    input_type: inputType,
    description,
    include_audio: Boolean(values.include_audio),
    record: values.record === undefined ? true : Boolean(values.record),
  };

  if (inputType === "xiaomi") {
    const xm = {
      user_id: String(values.xm_user_id ?? "").trim(),
      token: String(values.xm_token ?? "").trim(),
      region: String(values.xm_region ?? "").trim(),
      did: String(values.xm_did ?? "").trim(),
      model: String(values.xm_model ?? "").trim(),
      ip: String(values.xm_ip ?? "").trim(),
    };
    if (!xm.user_id) errors.xm_user_id = [{ message: "请输入用户 ID" }];
    if (!xm.token) errors.xm_token = [{ message: "请输入 Token" }];
    if (!xm.did) errors.xm_did = [{ message: "请输入设备 DID" }];
    if (!xm.model) errors.xm_model = [{ message: "请输入设备型号" }];
    if (!xm.ip) errors.xm_ip = [{ message: "请输入摄像头 IP" }];

    Object.assign(cleaned, {
      xm_user_id: xm.user_id,
      xm_token: xm.token,
      xm_region: xm.region,
      xm_did: xm.did,
      xm_model: xm.model,
      xm_ip: xm.ip,
      input_value: JSON.stringify(xm),
    });
  } else if (inputType === "gb28181") {
    if (!gbDeviceId.value) {
      errors.input_value = [{ message: "请选择国标设备" }];
    } else if (!gbChannelId.value) {
      errors.input_value = [{ message: "请选择国标通道" }];
    }
    cleaned.input_value = JSON.stringify({
      device_id: gbDeviceId.value,
      channel_id: gbChannelId.value,
    });
  } else {
    const inputValue = String(values.input_value ?? "").trim();
    if (!inputValue) {
      errors.input_value = [{ message: "请输入输入地址或标识" }];
    }
    cleaned.input_value = inputValue;
  }

  return { values: cleaned, errors };
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
  resetGbFields();
  dialogVisible.value = true;
}

function openEditDialog(device: DeviceItem) {
  editingDevice.value = device;
  resetGbFields();
  hydrateGbFields(device);
  dialogVisible.value = true;
}

// Split a gb28181 device's input_value JSON back into the picker refs and
// preload its device list + channel catalog so the dropdowns show the saved
// selection when editing.
function hydrateGbFields(device: DeviceItem) {
  if (device.input_type !== "gb28181" || !device.input_value) {
    return;
  }
  try {
    const cfg = JSON.parse(device.input_value) as {
      device_id?: string;
      channel_id?: string;
    };
    gbDeviceId.value = cfg.device_id ?? "";
    gbChannelId.value = cfg.channel_id ?? "";
    void loadGbDevices();
    if (gbDeviceId.value) {
      const hydratedDeviceId = gbDeviceId.value;
      const savedChannel = gbChannelId.value;
      void onGbDeviceChange(hydratedDeviceId).then(() => {
        // Only restore the saved channel if the dialog is still on this device
        // (a rapid edit→edit on a different device must not clobber it).
        if (gbDeviceId.value === hydratedDeviceId) {
          gbChannelId.value = savedChannel;
        }
      });
    }
  } catch {
    resetGbFields();
  }
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

  const inputType = String(event.values.input_type ?? "");
  // The gb pickers are standalone refs (not @primevue/forms fields), so guard
  // their required-ness here rather than relying on the resolver's render-order
  // side-effect to block the submit.
  if (inputType === "gb28181" && (!gbDeviceId.value || !gbChannelId.value)) {
    return;
  }
  let inputValue = String(event.values.input_value ?? "");
  if (inputType === "xiaomi") {
    inputValue = JSON.stringify({
      user_id: String(event.values.xm_user_id ?? "").trim(),
      token: String(event.values.xm_token ?? "").trim(),
      region: String(event.values.xm_region ?? "").trim(),
      did: String(event.values.xm_did ?? "").trim(),
      model: String(event.values.xm_model ?? "").trim(),
      ip: String(event.values.xm_ip ?? "").trim(),
    });
  } else if (inputType === "gb28181") {
    inputValue = JSON.stringify({
      device_id: gbDeviceId.value,
      channel_id: gbChannelId.value,
    });
  }

  const payload: DevicePayload = {
    name: String(event.values.name ?? ""),
    input_type: inputType,
    input_value: inputValue,
    description: String(event.values.description ?? ""),
    include_audio: Boolean(event.values.include_audio),
    record: event.values.record === undefined ? true : Boolean(event.values.record),
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

// Never expose a xiaomi device's raw input_value (it holds the secret token) in
// the table — show a redacted did/model/ip summary instead.
function inputValueDisplay(device: DeviceItem) {
  if (device.input_type === "gb28181") {
    try {
      const cfg = JSON.parse(device.input_value) as {
        device_id?: string;
        channel_id?: string;
      };
      return `${cfg.device_id ?? "?"} / ${cfg.channel_id ?? "?"}`;
    } catch {
      return device.input_value;
    }
  }
  if (device.input_type !== "xiaomi") {
    return device.input_value;
  }
  try {
    const cfg = JSON.parse(device.input_value) as Partial<
      Record<"did" | "model" | "ip", string>
    >;
    const parts = [
      cfg.did && `did=${cfg.did}`,
      cfg.model && `model=${cfg.model}`,
      cfg.ip && `ip=${cfg.ip}`,
    ].filter(Boolean);
    return parts.length ? parts.join("  ") : "小米摄像头";
  } catch {
    return "小米摄像头";
  }
}

function buildFlvUrl(deviceId: string) {
  // Same-origin path through the `/media` reverse proxy, not ZLM's direct
  // 127.0.0.1:8553 — so playback works behind port-forwarding / remote access.
  return `/media/live/${encodeURIComponent(deviceId)}.live.flv`;
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
              <div class="copy-cell copy-cell-input" :title="inputValueDisplay(data)">
                <span class="mono-text ellipsis-text">{{ inputValueDisplay(data) }}</span>
                <Button
                  icon="pi pi-copy"
                  text
                  rounded
                  class="copy-button"
                  aria-label="复制输入地址"
                  @click="copyText(inputValueDisplay(data), '输入地址')"
                />
              </div>
            </template>
          </Column>
          <Column field="updated_at" header="更新时间" style="width: 12rem; min-width: 12rem">
            <template #body="{ data }">
              {{ formatTime(data.updated_at) }}
            </template>
          </Column>
          <Column header="状态" style="width: 6rem">
            <template #body="{ data }">
              <Tag
                v-if="data.input_type === 'gb28181'"
                :value="gbStreamStatus[data.id]?.live ? '拉流中' : '空闲'"
                :severity="gbStreamStatus[data.id]?.live ? 'success' : 'secondary'"
              />
            </template>
          </Column>
          <Column
            header="操作"
            :exportable="false"
            class="action-column"
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
                  v-if="data.input_type === 'gb28181'"
                  icon="pi pi-compass"
                  text
                  rounded
                  aria-label="云台"
                  title="云台控制"
                  @click="openPtzDialog(data)"
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
        :key="editingDevice?.id ?? 'new'"
        :resolver="resolver"
        :initial-values="formInitialValues"
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

        <template v-if="$form.input_type?.value === 'xiaomi'">
          <div class="field-grid">
            <div class="field">
              <label for="xm_user_id">用户 ID</label>
              <InputText
                id="xm_user_id"
                name="xm_user_id"
                class="field-input"
                placeholder="Xiaomi user_id"
                :invalid="$form.xm_user_id?.invalid"
              />
              <Message
                v-if="$form.xm_user_id?.invalid"
                severity="error"
                size="small"
                variant="simple"
              >
                {{ $form.xm_user_id.error?.message }}
              </Message>
            </div>
            <div class="field">
              <label for="xm_token">Token</label>
              <Password
                id="xm_token"
                name="xm_token"
                class="field-input"
                :feedback="false"
                toggle-mask
                placeholder="passToken"
                :invalid="$form.xm_token?.invalid"
              />
              <Message
                v-if="$form.xm_token?.invalid"
                severity="error"
                size="small"
                variant="simple"
              >
                {{ $form.xm_token.error?.message }}
              </Message>
            </div>
          </div>

          <div class="field-grid">
            <div class="field">
              <label for="xm_did">设备 DID</label>
              <InputText
                id="xm_did"
                name="xm_did"
                class="field-input"
                placeholder="did"
                :invalid="$form.xm_did?.invalid"
              />
              <Message v-if="$form.xm_did?.invalid" severity="error" size="small" variant="simple">
                {{ $form.xm_did.error?.message }}
              </Message>
            </div>
            <div class="field">
              <label for="xm_model">型号 Model</label>
              <InputText
                id="xm_model"
                name="xm_model"
                class="field-input"
                placeholder="如 chuangmi.camera.xxx"
                :invalid="$form.xm_model?.invalid"
              />
              <Message
                v-if="$form.xm_model?.invalid"
                severity="error"
                size="small"
                variant="simple"
              >
                {{ $form.xm_model.error?.message }}
              </Message>
            </div>
          </div>

          <div class="field-grid">
            <div class="field">
              <label for="xm_ip">摄像头 IP</label>
              <InputText
                id="xm_ip"
                name="xm_ip"
                class="field-input"
                placeholder="192.168.x.y"
                :invalid="$form.xm_ip?.invalid"
              />
              <Message v-if="$form.xm_ip?.invalid" severity="error" size="small" variant="simple">
                {{ $form.xm_ip.error?.message }}
              </Message>
            </div>
            <div class="field">
              <label for="xm_region">区域</label>
              <Select
                id="xm_region"
                name="xm_region"
                :options="regionOptions"
                option-label="label"
                option-value="value"
                size="small"
                class="field-input"
                placeholder="选择区域"
              />
            </div>
          </div>

          <div class="field">
            <span class="field-hint">
              运行 <code>cargo run -p xiaomi --bin validate</code>
              登录小米账号即可获取 user_id / token，并列出各摄像头的 did / model / ip。
            </span>
          </div>
        </template>

        <template v-else-if="$form.input_type?.value === 'gb28181'">
          <div class="field">
            <label for="gb_device">国标设备</label>
            <Select
              id="gb_device"
              class="field-input"
              :options="gbDevices"
              option-label="device_id"
              option-value="device_id"
              :model-value="gbDeviceId"
              placeholder="选择已注册的国标设备"
              @update:model-value="onGbDeviceChange"
              @before-show="loadGbDevices"
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
              v-model="gbChannelId"
              class="field-input"
              :options="gbChannels"
              option-label="name"
              option-value="channel_id"
              placeholder="选择通道"
              :disabled="!gbDeviceId"
            />
            <Message
              v-if="$form.input_value?.invalid"
              severity="error"
              size="small"
              variant="simple"
            >
              {{ $form.input_value.error?.message }}
            </Message>
          </div>
          <div class="field">
            <span class="field-hint">
              国标设备需先注册到本平台（NVR_GB_ENABLE=1）。下拉框列出已注册设备（离线设备会标注），
              选择后加载其通道，保存后仅在有人观看时按需 INVITE 拉流。
            </span>
          </div>
        </template>

        <div v-else class="field">
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

        <div class="field field-inline">
          <label for="include_audio">包含音频</label>
          <ToggleSwitch id="include_audio" name="include_audio" />
          <span class="field-hint">开启后推流会包含音频轨（需输入源带音频）</span>
        </div>

        <div class="field field-inline">
          <label for="record">录制</label>
          <ToggleSwitch id="record" name="record" />
          <span class="field-hint">开启后该设备的录像会保存到磁盘，可在回放中查看</span>
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
        <TranscriptPanel
          v-if="previewDevice"
          :key="previewDevice.id"
          :pipe-id="previewDevice.id"
        />
      </div>
    </Dialog>

    <Dialog
      v-model:visible="ptzDialogVisible"
      modal
      header="云台控制"
      :style="{ width: 'min(22rem, calc(100vw - 2rem))' }"
      @hide="ptzRelease"
    >
      <div class="ptz-pad">
        <span />
        <Button
          icon="pi pi-arrow-up"
          aria-label="上"
          @pointerdown="ptzPress('up')"
          @pointerup="ptzRelease"
          @pointerleave="ptzRelease"
          @pointercancel="ptzRelease"
        />
        <span />
        <Button
          icon="pi pi-arrow-left"
          aria-label="左"
          @pointerdown="ptzPress('left')"
          @pointerup="ptzRelease"
          @pointerleave="ptzRelease"
          @pointercancel="ptzRelease"
        />
        <Button icon="pi pi-stop" severity="secondary" aria-label="停止" @click="sendPtz('stop')" />
        <Button
          icon="pi pi-arrow-right"
          aria-label="右"
          @pointerdown="ptzPress('right')"
          @pointerup="ptzRelease"
          @pointerleave="ptzRelease"
          @pointercancel="ptzRelease"
        />
        <span />
        <Button
          icon="pi pi-arrow-down"
          aria-label="下"
          @pointerdown="ptzPress('down')"
          @pointerup="ptzRelease"
          @pointerleave="ptzRelease"
          @pointercancel="ptzRelease"
        />
        <span />
      </div>

      <div class="ptz-row">
        <Button
          label="放大"
          size="small"
          icon="pi pi-search-plus"
          @pointerdown="ptzPress('zoom_in')"
          @pointerup="ptzRelease"
          @pointerleave="ptzRelease"
          @pointercancel="ptzRelease"
        />
        <Button
          label="缩小"
          size="small"
          icon="pi pi-search-minus"
          @pointerdown="ptzPress('zoom_out')"
          @pointerup="ptzRelease"
          @pointerleave="ptzRelease"
          @pointercancel="ptzRelease"
        />
      </div>

      <div class="ptz-row">
        <label class="ptz-label">速度</label>
        <Slider v-model="ptzSpeed" :min="1" :max="255" class="ptz-slider" />
        <span class="ptz-value">{{ ptzSpeed }}</span>
      </div>

      <div class="ptz-row">
        <label class="ptz-label">预置位</label>
        <InputNumber
          v-model="ptzPreset"
          :min="1"
          :max="255"
          show-buttons
          :input-style="{ width: '3.5rem' }"
        />
        <Button label="调用" size="small" @click="sendPtz('preset_call', ptzPreset)" />
        <Button label="设置" size="small" severity="secondary" @click="sendPtz('preset_set', ptzPreset)" />
        <Button label="删除" size="small" text severity="danger" @click="sendPtz('preset_delete', ptzPreset)" />
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

.ptz-pad {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 0.5rem;
  justify-items: center;
  margin-bottom: 1rem;
}

.ptz-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.75rem;
}

.ptz-label {
  min-width: 3.5rem;
  color: #cbd5e1;
  font-size: 0.75rem;
  font-weight: 600;
}

.ptz-slider {
  flex: 1;
}

.ptz-value {
  min-width: 2.5rem;
  text-align: right;
  color: #94a3b8;
  font-size: 0.8125rem;
}

.device-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

:deep(.device-dialog .field-inline) {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.75rem;
}

:deep(.device-dialog .field-inline label) {
  margin-right: 0.25rem;
}

.field-hint {
  flex: 1 1 100%;
  font-size: 0.75rem;
  color: #94a3b8;
}

.gb-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  width: 100%;
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
