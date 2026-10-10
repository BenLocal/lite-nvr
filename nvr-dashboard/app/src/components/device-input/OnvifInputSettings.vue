<script setup lang="ts">
import { computed, ref } from 'vue'
import Button from 'primevue/button'
import InputNumber from 'primevue/inputnumber'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import Password from 'primevue/password'
import Select from 'primevue/select'
import { discoverOnvif, probeOnvif, type OnvifDiscovered, type OnvifProbe } from '../../api/onvif'
import type { DeviceFormFields } from '../../forms/deviceForm'
import { useAppToast } from '../../utils/toast'

const props = defineProps<{ form: DeviceFormFields }>()
// Field writes go through the page that owns the form, not the prop.
const emit = defineEmits<{ 'set-field': [name: string, value: unknown] }>()

const appToast = useAppToast()

// Probe/discover results load on demand from the camera itself, so they live
// here rather than in @primevue/forms fields.
const probing = ref(false)
const probe = ref<OnvifProbe | null>(null)
const discovering = ref(false)
const discovered = ref<OnvifDiscovered[]>([])

const profileOptions = computed(() =>
  (probe.value?.profiles ?? []).map((profile) => ({
    label: `${profile.name} (${profile.width}x${profile.height})`,
    value: profile.token,
  })),
)

// Parse "host:port" (or a bare host) out of a discovered device's addr.
function parseAddr(addr: string | null): { host: string; port: number } {
  if (!addr) return { host: '', port: 80 }
  const match = /^(.+):(\d+)$/.exec(addr)
  const host = match?.[1]
  const port = match?.[2]
  if (host && port) return { host, port: Number(port) }
  return { host: addr, port: 80 }
}

async function runProbe() {
  const form = props.form
  const host = String(form.onvif_host?.value ?? '').trim()
  const port = Number(form.onvif_port?.value ?? 80)
  if (!host) {
    appToast.warn('请先填写主机地址', undefined, 2000)
    return
  }
  probing.value = true
  try {
    const result = await probeOnvif({
      host,
      port,
      username: String(form.onvif_username?.value ?? '').trim(),
      password: String(form.onvif_password?.value ?? '').trim(),
    })
    probe.value = result
    const firstProfile = result.profiles[0]
    if (firstProfile) emit('set-field', 'onvif_profile_token', firstProfile.token)
    appToast.success('探测成功', `${result.device_info.manufacturer} ${result.device_info.model}`)
  } catch (error) {
    probe.value = null
    appToast.errorFrom('探测失败', error, '无法连接到 ONVIF 设备')
  } finally {
    probing.value = false
  }
}

async function runDiscover() {
  discovering.value = true
  try {
    discovered.value = await discoverOnvif()
    if (!discovered.value.length) {
      appToast.info('未发现设备', '局域网内未发现 ONVIF 设备', 2000)
    }
  } catch (error) {
    discovered.value = []
    appToast.errorFrom('扫描失败', error, '局域网扫描失败')
  } finally {
    discovering.value = false
  }
}

function applyDiscovered(item: OnvifDiscovered) {
  const { host, port } = parseAddr(item.addr)
  emit('set-field', 'onvif_host', host)
  emit('set-field', 'onvif_port', port)
}
</script>

<template>
  <div class="field-grid">
    <div class="field">
      <label for="onvif_host">主机地址</label>
      <InputText
        id="onvif_host"
        name="onvif_host"
        class="field-input"
        placeholder="192.168.x.y"
        :invalid="form.onvif_host?.invalid"
      />
      <Message v-if="form.onvif_host?.invalid" severity="error" size="small" variant="simple">
        {{ form.onvif_host.error?.message }}
      </Message>
    </div>
    <div class="field">
      <label for="onvif_port">端口</label>
      <InputNumber
        id="onvif_port"
        name="onvif_port"
        class="field-input"
        :use-grouping="false"
        :min="1"
        :max="65535"
      />
    </div>
  </div>

  <div class="field-grid">
    <div class="field">
      <label for="onvif_username">用户名</label>
      <InputText id="onvif_username" name="onvif_username" class="field-input" placeholder="admin" />
    </div>
    <div class="field">
      <label for="onvif_password">密码</label>
      <Password
        id="onvif_password"
        name="onvif_password"
        class="field-input"
        :feedback="false"
        toggle-mask
      />
    </div>
  </div>

  <div class="field-actions">
    <Button
      type="button"
      label="探测"
      icon="pi pi-search"
      size="small"
      :loading="probing"
      @click="runProbe"
    />
    <Button
      type="button"
      label="扫描局域网"
      icon="pi pi-wifi"
      size="small"
      severity="secondary"
      :loading="discovering"
      @click="runDiscover"
    />
  </div>

  <div v-if="discovered.length" class="field">
    <label>发现的设备</label>
    <ul class="onvif-discovered-list">
      <li v-for="item in discovered" :key="item.addr ?? item.endpoints[0]">
        <Button
          type="button"
          text
          size="small"
          class="onvif-discovered-item"
          @click="applyDiscovered(item)"
        >
          <span class="mono-text">{{ item.addr ?? "未知地址" }}</span>
          <span v-if="item.name" class="field-hint">{{ item.name }}</span>
        </Button>
      </li>
    </ul>
  </div>

  <div v-if="probe" class="field">
    <label>设备信息</label>
    <div class="onvif-device-info">
      <span>{{ probe.device_info.manufacturer }} {{ probe.device_info.model }}</span>
      <span class="field-hint">固件 {{ probe.device_info.firmware }}</span>
    </div>
  </div>

  <!--
    v-show (not v-if): the field must stay registered with the form even before
    a probe succeeds, otherwise `form.onvif_profile_token` does not exist yet
    when runProbe() asks the page to set the auto-selected profile.
  -->
  <div v-show="probe" class="field">
    <label for="onvif_profile_token">视频配置文件</label>
    <Select
      id="onvif_profile_token"
      name="onvif_profile_token"
      :options="profileOptions"
      option-label="label"
      option-value="value"
      size="small"
      class="field-input"
      placeholder="请选择视频配置文件"
    />
  </div>

  <!-- Outside the v-show block: "probe first" must show before any probe. -->
  <Message v-if="form.input_value?.invalid" severity="error" size="small" variant="simple">
    {{ form.input_value.error?.message }}
  </Message>

  <div class="field">
    <span class="field-hint">
      填写摄像头 ONVIF 服务地址、端口及登录凭据后点击“探测”获取设备信息与可用的视频配置文件；
      也可点击“扫描局域网”自动发现同网段内的 ONVIF 设备并预填地址。
    </span>
  </div>
</template>

<style scoped>
.onvif-discovered-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  margin: 0;
  padding: 0;
  list-style: none;
}

.onvif-discovered-item {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 0.125rem;
  width: 100%;
  text-align: left;
}

.onvif-device-info {
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
  font-size: 0.8125rem;
  color: #e2e8f0;
}
</style>
