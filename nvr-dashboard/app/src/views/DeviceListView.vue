<script setup lang="ts">
/* eslint-disable @typescript-eslint/no-unused-vars -- bindings are consumed by the external SFC template. */
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
import Tab from "primevue/tab";
import TabList from "primevue/tablist";
import TabPanel from "primevue/tabpanel";
import TabPanels from "primevue/tabpanels";
import Tabs from "primevue/tabs";
import Tag from "primevue/tag";
import Textarea from "primevue/textarea";
import ToggleSwitch from "primevue/toggleswitch";
import { useConfirm } from "primevue/useconfirm";
import FlvPreviewPlayer from "../components/FlvPreviewPlayer.vue";
import DetectionConfigFields from "../components/DetectionConfigFields.vue";
import TranscriptPanel from "../components/TranscriptPanel.vue";
import {
  addDevice,
  listDevices,
  removeDevice,
  updateDevice,
  type DeviceItem,
} from "../api/device";
import { getDetectionCapabilities } from "../api/detect";
import {
  buildDevicePayload,
  deviceFormInitialValues,
  inputTypeOptions,
  resolveDeviceForm,
  xiaomiRegionOptions,
} from "../forms/deviceForm";
import {
  getGbCatalog,
  getGbDevices,
  getGbStreams,
  ptzControl,
  type GbChannel,
  type GbDevice,
  type GbStream,
} from "../api/gb";
import {
  discoverOnvif,
  getOnvifPresets,
  onvifPtz,
  probeOnvif,
  type OnvifDiscovered,
  type OnvifPreset,
  type OnvifProbe,
} from "../api/onvif";
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

const detectModelOptions = ref<string[]>([]);
const detectSupportedInputTypes = ref<string[]>([]);
const detectMaxSampleIntervalMs = ref(0);
const detectCapabilityStatus = ref<"loading" | "ready" | "error">("loading");
const detectCapabilityError = ref("");

function detectSupported(inputType?: string): boolean {
  return (
    !!inputType &&
    (detectCapabilityStatus.value !== "ready" ||
      detectSupportedInputTypes.value.includes(inputType))
  );
}

async function loadDetectionCapabilities() {
  detectCapabilityStatus.value = "loading";
  detectCapabilityError.value = "";
  try {
    const capabilities = await getDetectionCapabilities();
    detectModelOptions.value = [...capabilities.models];
    detectSupportedInputTypes.value = [...capabilities.supported_input_types];
    detectMaxSampleIntervalMs.value = capabilities.max_sample_interval_ms;
    detectCapabilityStatus.value = "ready";
  } catch (error) {
    detectModelOptions.value = [];
    detectSupportedInputTypes.value = [];
    detectCapabilityError.value = error instanceof Error ? error.message : String(error);
    detectCapabilityStatus.value = "error";
  }
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

// ONVIF probe/discover state. Standalone refs (not @primevue/forms fields)
// because the profile list is loaded on demand from the camera itself.
const onvifProbing = ref(false);
const onvifProbe = ref<OnvifProbe | null>(null);
const onvifDiscovering = ref(false);
const onvifDiscovered = ref<OnvifDiscovered[]>([]);

const onvifProfileOptions = computed(() => {
  const profiles = onvifProbe.value?.profiles ?? [];
  return profiles.map((profile) => ({
    label: `${profile.name} (${profile.width}x${profile.height})`,
    value: profile.token,
  }));
});

// Parse "host:port" (or a bare host) out of a discovered device's addr and
// prefill the host/port form fields.
function parseOnvifAddr(addr: string | null): { host: string; port: number } {
  if (!addr) return { host: "", port: 80 };
  const match = /^(.+):(\d+)$/.exec(addr);
  const host = match?.[1];
  const port = match?.[2];
  if (host && port) {
    return { host, port: Number(port) };
  }
  return { host: addr, port: 80 };
}

function resetOnvifFields() {
  onvifProbing.value = false;
  onvifProbe.value = null;
  onvifDiscovering.value = false;
  onvifDiscovered.value = [];
}

// The `$form` slot from @primevue/forms exposes each field's reactive
// FormFieldState directly (v-bind="states" in Form.vue) but does NOT expose
// setFieldValue/setValues on the slot scope — those only exist on the
// useForm() instance. Writing `.value` on the field state is the same
// mutation setFieldValue performs internally, so it's the correct way to set
// a field's value programmatically from the default slot. This type mirrors
// the slot's own `{ [key: string]: FormFieldState }` shape so it accepts
// `$form` directly with no cast.
type OnvifFormFields = Record<string, { value: unknown } | undefined>;

async function runOnvifProbe(form: OnvifFormFields) {
  const host = String(form.onvif_host?.value ?? "").trim();
  const port = Number(form.onvif_port?.value ?? 80);
  if (!host) {
    appToast.warn("请先填写主机地址", undefined, 2000);
    return;
  }
  onvifProbing.value = true;
  try {
    const result = await probeOnvif({
      host,
      port,
      username: String(form.onvif_username?.value ?? "").trim(),
      password: String(form.onvif_password?.value ?? "").trim(),
    });
    onvifProbe.value = result;
    const firstProfile = result.profiles[0];
    if (firstProfile && form.onvif_profile_token) {
      form.onvif_profile_token.value = firstProfile.token;
    }
    appToast.success("探测成功", `${result.device_info.manufacturer} ${result.device_info.model}`);
  } catch (error) {
    onvifProbe.value = null;
    appToast.errorFrom("探测失败", error, "无法连接到 ONVIF 设备");
  } finally {
    onvifProbing.value = false;
  }
}

async function runOnvifDiscover() {
  onvifDiscovering.value = true;
  try {
    onvifDiscovered.value = await discoverOnvif();
    if (!onvifDiscovered.value.length) {
      appToast.info("未发现设备", "局域网内未发现 ONVIF 设备", 2000);
    }
  } catch (error) {
    onvifDiscovered.value = [];
    appToast.errorFrom("扫描失败", error, "局域网扫描失败");
  } finally {
    onvifDiscovering.value = false;
  }
}

function applyOnvifDiscovered(item: OnvifDiscovered, form: OnvifFormFields) {
  const { host, port } = parseOnvifAddr(item.addr);
  if (form.onvif_host) form.onvif_host.value = host;
  if (form.onvif_port) form.onvif_port.value = port;
}

const regionOptions = xiaomiRegionOptions;
const formInitialValues = computed(() => deviceFormInitialValues(editingDevice.value));

onMounted(() => {
  void loadDevices();
  loadGbStreams();
  gbTimer = setInterval(loadGbStreams, 5000);
  void loadDetectionCapabilities();
});

onUnmounted(() => {
  if (gbTimer) clearInterval(gbTimer);
});

function resolver(event: { values: Record<string, unknown> }) {
  const inputType = String(event.values.input_type ?? "");
  return resolveDeviceForm(event, {
    gbDeviceId: gbDeviceId.value,
    gbChannelId: gbChannelId.value,
    validateDetection:
      detectCapabilityStatus.value === "ready" && detectSupported(inputType),
    maxDetectionSampleIntervalMs: detectMaxSampleIntervalMs.value,
  });
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
  resetOnvifFields();
  dialogVisible.value = true;
}

function openEditDialog(device: DeviceItem) {
  editingDevice.value = device;
  resetGbFields();
  resetOnvifFields();
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
  if (!event.valid) return;
  const inputType = String(event.values.input_type ?? "");
  if (inputType === "gb28181" && (!gbDeviceId.value || !gbChannelId.value)) {
    return;
  }
  if (Boolean(event.values.detect_enabled) && detectCapabilityStatus.value !== "ready") {
    appToast.warn("检测能力尚未就绪", "请重试能力加载后再启用检测");
    return;
  }
  const payload = buildDevicePayload(event.values, {
    gbDeviceId: gbDeviceId.value,
    gbChannelId: gbChannelId.value,
    supportedDetectionInputTypes: detectSupportedInputTypes.value,
  });
  await saveDevice(payload);
}

async function saveDevice(payload: ReturnType<typeof buildDevicePayload>) {
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
