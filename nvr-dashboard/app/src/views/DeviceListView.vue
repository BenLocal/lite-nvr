<script setup lang="ts">
/* eslint-disable @typescript-eslint/no-unused-vars -- bindings are consumed by the external SFC template. */
import { onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import Button from "primevue/button";
import Card from "primevue/card";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import Menu from "primevue/menu";
import type { MenuItem } from "primevue/menuitem";
import InputNumber from "primevue/inputnumber";
import Select from "primevue/select";
import Slider from "primevue/slider";
import Tag from "primevue/tag";
import { useConfirm } from "primevue/useconfirm";
import FlvPreviewPlayer from "../components/FlvPreviewPlayer.vue";
import DeviceFlowTopology from "../components/DeviceFlowTopology.vue";
import Message from "primevue/message";
import TranscriptPanel from "../components/TranscriptPanel.vue";
import { listDevices, removeDevice, getDeviceMetrics, type DeviceItem, type DeviceMetrics } from "../api/device";
import { ptzControl } from "../api/gb";
import { useAppToast } from "../utils/toast";

const appToast = useAppToast();
const confirm = useConfirm();
const router = useRouter();

const loading = ref(false);
const devices = ref<DeviceItem[]>([]);
const actionsMenu = ref<InstanceType<typeof Menu> | null>(null);
const actionItems = ref<MenuItem[]>([]);
function openActions(event: Event, device: DeviceItem) {
  actionItems.value = [
    { label: "流拓扑", icon: "pi pi-share-alt", command: () => openMetrics(device) },
    ...(device.input_type === "gb28181" ? [{ label: "云台", icon: "pi pi-compass", command: () => openPtzDialog(device) }] : []),
    { label: "编辑", icon: "pi pi-pencil", command: () => openEditPage(device) },
    { separator: true },
    { label: "删除", icon: "pi pi-trash", class: "device-menu-delete", command: () => confirmDelete(device) },
  ];
  actionsMenu.value?.toggle(event);
}
const metricsVisible = ref(false);
const metricsDevice = ref<DeviceItem | null>(null);
const deviceMetrics = ref<Record<string, DeviceMetrics>>({});
const metricsError = ref<string | null>(null);
let metricsTimer: ReturnType<typeof setTimeout> | undefined;
let metricsController: AbortController | undefined;
let disposed = false;

async function pollMetrics() {
  if (disposed || !metricsVisible.value) return;
  const controller = new AbortController();
  metricsController = controller;
  if (document.visibilityState !== "hidden") {
    const deadline = setTimeout(() => controller.abort(), 5000);
    try {
      const samples = await getDeviceMetrics(controller.signal);
      if (!disposed && metricsVisible.value && metricsController === controller) {
        deviceMetrics.value = Object.fromEntries(samples.map(sample => [sample.id, sample]));
        metricsError.value = null;
      }
    } catch (error) {
      if (!disposed && metricsVisible.value && metricsController === controller) metricsError.value = error instanceof Error ? error.message : "流量数据更新失败";
    } finally {
      clearTimeout(deadline);
    }
  }
  if (!disposed && metricsVisible.value && metricsController === controller) metricsTimer = setTimeout(() => { void pollMetrics(); }, 2000);
}
watch(metricsVisible, visible => {
  if (metricsTimer) clearTimeout(metricsTimer);
  metricsController?.abort();
  metricsController = undefined;
  if (visible) {
    metricsError.value = null;
    void pollMetrics();
  }
});

function openMetrics(device: DeviceItem) {
  metricsDevice.value = device;
  metricsVisible.value = true;
}

const previewVisible = ref(false);
const previewDevice = ref<DeviceItem | null>(null);

let devicesTimer: ReturnType<typeof setInterval> | undefined;
let devicesRefreshInFlight = false;

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

onMounted(() => {
  void loadDevices();
  devicesTimer = setInterval(() => {
    if (document.visibilityState !== "hidden") void loadDevices(true);
  }, 5000);
});

onUnmounted(() => {
  if (devicesTimer) clearInterval(devicesTimer);
  disposed = true;
  if (metricsTimer) clearTimeout(metricsTimer);
  metricsController?.abort();
});

async function loadDevices(silent = false) {
  if (devicesRefreshInFlight || disposed) return;
  devicesRefreshInFlight = true;
  if (!silent) loading.value = true;
  try {
    const list = await listDevices();
    if (!disposed) devices.value = list;
  } catch (error) {
    if (!silent && !disposed) appToast.errorFrom("加载失败", error, "设备列表加载失败");
  } finally {
    devicesRefreshInFlight = false;
    loading.value = false;
  }
}

function openCreatePage() {
  void router.push({ name: "device-create" });
}

function openEditPage(device: DeviceItem) {
  void router.push({ name: "device-edit", params: { id: device.id } });
}

function openPreview(device: DeviceItem) {
  previewDevice.value = device;
  previewVisible.value = true;
}

function closePreview() {
  previewVisible.value = false;
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
  if (device.input_type === "onvif") {
    try {
      const cfg = JSON.parse(device.input_value) as {
        host?: string;
        port?: number;
        profile_token?: string;
      };
      const addr = cfg.host ? `${cfg.host}:${cfg.port ?? 80}` : "ONVIF";
      return cfg.profile_token ? `${addr} (${cfg.profile_token})` : addr;
    } catch {
      return "ONVIF 摄像头";
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
  return `/media/device/${encodeURIComponent(deviceId)}.live.flv`;
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

<template src="./DeviceListView.template.html"></template>

<style scoped src="./DeviceListView.css"></style>
