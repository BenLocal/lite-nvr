<script setup lang="ts">
/* eslint-disable @typescript-eslint/no-unused-vars -- bindings are consumed by the external SFC template. */
import { onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import Button from "primevue/button";
import Card from "primevue/card";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import InputNumber from "primevue/inputnumber";
import Select from "primevue/select";
import Slider from "primevue/slider";
import Tag from "primevue/tag";
import { useConfirm } from "primevue/useconfirm";
import FlvPreviewPlayer from "../components/FlvPreviewPlayer.vue";
import TranscriptPanel from "../components/TranscriptPanel.vue";
import { listDevices, removeDevice, type DeviceItem } from "../api/device";
import { getGbStreams, ptzControl, type GbStream } from "../api/gb";
import { getOnvifPresets, onvifPtz, type OnvifPreset } from "../api/onvif";
import { useAppToast } from "../utils/toast";

const appToast = useAppToast();
const confirm = useConfirm();
const router = useRouter();

const loading = ref(false);
const devices = ref<DeviceItem[]>([]);
const previewVisible = ref(false);
const previewDevice = ref<DeviceItem | null>(null);

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

// PTZ (云台) control for onvif devices. The target is simply the device's own
// id (ONVIF connection config is registered under that same id server-side);
// moves are press-and-hold (move on press, stop on release/leave) and presets
// are selected from a dropdown populated on open, mirroring the gb28181 block above.
const onvifPtzDialogVisible = ref(false);
const onvifPtzTarget = ref<string | null>(null);
const onvifPtzSpeed = ref(128);
const onvifPtzPresets = ref<OnvifPreset[]>([]);
const onvifPtzPresetToken = ref<string>("");

function openOnvifPtzDialog(device: DeviceItem) {
  onvifPtzTarget.value = device.id;
  onvifPtzPresetToken.value = "";
  onvifPtzDialogVisible.value = true;
}

async function loadOnvifPtzPresets() {
  if (!onvifPtzTarget.value) return;
  try {
    onvifPtzPresets.value = await getOnvifPresets(onvifPtzTarget.value);
  } catch {
    onvifPtzPresets.value = [];
  }
}

async function sendOnvifPtz(direction: string, presetToken?: string) {
  if (!onvifPtzTarget.value) return;
  try {
    await onvifPtz({
      device_id: onvifPtzTarget.value,
      direction,
      speed: onvifPtzSpeed.value,
      preset_token: presetToken,
    });
  } catch {
    // Best-effort: a failed PTZ command shouldn't disrupt the UI. A dropped
    // move still gets a stop on release, so the camera won't run away.
  }
}

// Press-and-hold: send the move on press, stop on release/leave/cancel.
function onvifPtzPress(direction: string) {
  void sendOnvifPtz(direction);
}

function onvifPtzRelease() {
  void sendOnvifPtz("stop");
}

onMounted(() => {
  void loadDevices();
  loadGbStreams();
  gbTimer = setInterval(loadGbStreams, 5000);
});

onUnmounted(() => {
  if (gbTimer) clearInterval(gbTimer);
});

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
