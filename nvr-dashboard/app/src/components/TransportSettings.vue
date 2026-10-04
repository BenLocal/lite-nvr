<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import Form from '@primevue/forms/form'
import Button from 'primevue/button'
import Card from 'primevue/card'
import Column from 'primevue/column'
import DataTable from 'primevue/datatable'
import Dialog from 'primevue/dialog'
import InputNumber from 'primevue/inputnumber'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import MultiSelect from 'primevue/multiselect'
import Password from 'primevue/password'
import Select from 'primevue/select'
import ToggleSwitch from 'primevue/toggleswitch'
import { useConfirm } from 'primevue/useconfirm'
import { listDevices, type DeviceItem } from '../api/device'
import {
  getTransportCapabilities, listTransportTargets, removeTransportTarget,
  saveTransportTarget, testTransportTarget, type TransportPayload, type TransportTarget,
} from '../api/transport'
import { useAppToast } from '../utils/toast'

const toast = useAppToast()
const confirm = useConfirm()
const targets = ref<TransportTarget[]>([])
const devices = ref<DeviceItem[]>([])
const loading = ref(false)
const loadError = ref('')
const smbEnabled = ref(false)
const ready = ref(false)
const visible = ref(false)
const editingId = ref<string | null>(null)
const saving = ref(false)
const testingId = ref<string | null>(null)
const kind = ref<'ftp' | 'smb'>('ftp')
const enabled = ref(false)
const allDevices = ref(true)
const selectedDevices = ref<string[]>([])
const initialValues = ref({ name: '', host: '', port: 21, share: '', workgroup: 'WORKGROUP', username: '', password: '', base_path: '', remark: '' })
watch(kind, (value) => {
  if (value === 'smb' && !smbEnabled.value) enabled.value = false
})
const kindOptions = [{ label: 'FTP', value: 'ftp' }, { label: 'SMB', value: 'smb' }]
const deviceOptions = computed(() => {
  const options = devices.value.map((device) => ({ label: device.name || device.id, value: device.id }))
  for (const id of selectedDevices.value) {
    if (!options.some((option) => option.value === id)) options.push({ label: `${id}（已移除）`, value: id })
  }
  return options
})

async function load() {
  loading.value = true
  loadError.value = ''
  ready.value = false
  try {
    const [list, capability, deviceList] = await Promise.all([
      listTransportTargets(), getTransportCapabilities(), listDevices(),
    ])
    targets.value = list
    smbEnabled.value = capability.smb_enabled
    devices.value = deviceList
    ready.value = true
  } catch (error) {
    loadError.value = error instanceof Error ? error.message : '无法加载转存配置'
  } finally {
    loading.value = false
  }
}

function open(target?: TransportTarget) {
  editingId.value = target?.id ?? null
  kind.value = target?.kind ?? 'ftp'
  enabled.value = (target?.enabled ?? false) && (kind.value !== 'smb' || smbEnabled.value)
  allDevices.value = target?.config.stream_ids == null
  selectedDevices.value = [...(target?.config.stream_ids ?? [])]
  initialValues.value = {
    name: target?.name ?? '', host: target?.config.host ?? '', port: target?.config.port ?? 21,
    share: target?.config.share ?? '', workgroup: target?.config.workgroup ?? 'WORKGROUP',
    username: target?.config.username ?? '', password: '', base_path: target?.config.base_path ?? '',
    remark: target?.remark ?? '',
  }
  visible.value = true
}

function resolver({ values }: { values: Record<string, unknown> }) {
  const errors: Record<string, { message: string }[]> = {}
  for (const key of ['name', 'host']) {
    if (typeof values[key] !== 'string' || !values[key].trim()) errors[key] = [{ message: '此项必填' }]
  }
  if (kind.value === 'ftp' && (typeof values.port !== 'number' || !Number.isInteger(values.port) || values.port < 1 || values.port > 65535)) {
    errors.port = [{ message: '端口范围为 1–65535' }]
  }
  if (kind.value === 'smb' && (typeof values.share !== 'string' || !values.share.trim().replace(/\//g, ''))) {
    errors.share = [{ message: '请输入共享名称' }]
  }
  return { values, errors }
}

async function save(event: { valid: boolean; values: Record<string, unknown> }) {
  if (!event.valid) return
  if (enabled.value && kind.value === 'smb' && !smbEnabled.value) {
    toast.warn('无法启用', '当前服务器未启用 SMB 转存，请先关闭此目标')
    return
  }
  saving.value = true
  const values = event.values
  const payload: TransportPayload = {
    name: String(values.name).trim(), kind: kind.value, enabled: enabled.value,
    config: {
      host: String(values.host).trim(), username: String(values.username ?? ''),
      password: String(values.password ?? ''), base_path: String(values.base_path ?? '').trim(),
      ...(kind.value === 'ftp' ? { port: Number(values.port) }
        : { share: String(values.share).trim(), workgroup: String(values.workgroup ?? 'WORKGROUP') }),
      stream_ids: allDevices.value ? null : [...selectedDevices.value],
    },
    remark: String(values.remark ?? ''),
  }
  try {
    await saveTransportTarget(editingId.value, payload)
    visible.value = false
    toast.success('已保存', '录像转存配置已更新')
    await load()
  } catch (error) {
    toast.errorFrom('保存失败', error, '无法保存转存配置')
  } finally {
    saving.value = false
  }
}

async function test(target: TransportTarget) {
  testingId.value = target.id
  try {
    await testTransportTarget(target.id)
    toast.success('连接成功', target.name)
  } catch (error) {
    toast.errorFrom('连接失败', error, '无法连接转存目标')
  } finally {
    testingId.value = null
  }
}

function remove(target: TransportTarget) {
  confirm.require({
    header: '删除转存目标', message: `确认删除「${target.name}」及其转存任务记录？本地录像保留。`,
    icon: 'pi pi-exclamation-triangle', acceptLabel: '删除', rejectLabel: '取消',
    accept: async () => {
      try {
        await removeTransportTarget(target.id)
        toast.success('已删除', target.name)
        await load()
      } catch (error) {
        toast.errorFrom('删除失败', error, '无法删除转存目标')
      }
    },
  })
}

onMounted(load)
</script>

<template>
  <Card class="data-card transport-settings">
    <template #header>
      <div class="transport-toolbar">
        <i class="pi pi-upload transport-icon" /><span class="transport-title">录像转存</span>
        <Button label="新增目标" icon="pi pi-plus" :disabled="!ready || loading" @click="open()" />
        <Button label="刷新" icon="pi pi-refresh" text :loading="loading" @click="load" />
      </div>
    </template>
    <template #content>
      <Message v-if="loadError" severity="error">{{ loadError }}</Message>
      <p class="transport-hint">录像复制到远端，本地文件保留。启用目标后会补送所选设备的历史录像；失败最多自动尝试 5 次。</p>
      <DataTable :value="targets" :loading="loading" size="small">
        <template #empty>暂无转存目标</template>
        <Column field="name" header="名称" />
        <Column field="kind" header="协议" />
        <Column header="状态"><template #body="{ data }">{{ data.enabled ? '已启用' : '已停用' }}</template></Column>
        <Column header="设备范围"><template #body="{ data }">{{ data.config.stream_ids == null ? '全部设备' : `${data.config.stream_ids.length} 台设备` }}</template></Column>
        <Column field="done" header="已完成" />
        <Column field="failed" header="失败" />
        <Column header="操作">
          <template #body="{ data }">
            <div class="transport-toolbar">
              <Button label="编辑" text :disabled="!ready" @click="open(data)" />
              <Button label="测试连接" text :loading="testingId === data.id" :disabled="testingId !== null || !ready || (data.kind === 'smb' && !smbEnabled)" @click="test(data)" />
              <Button label="删除" text severity="danger" :disabled="!ready" @click="remove(data)" />
            </div>
          </template>
        </Column>
      </DataTable>
    </template>
  </Card>
  <Dialog v-model:visible="visible" modal :header="editingId ? '编辑转存目标' : '新增转存目标'" :closable="!saving" :close-on-escape="!saving" :style="{ width: '38rem', maxWidth: '95vw' }">
    <Form v-if="visible" v-slot="$form" :initial-values="initialValues" :resolver="resolver" class="transport-form" @submit="save">
      <div class="field">
        <label for="transport-name">名称</label>
        <InputText id="transport-name" name="name" class="field-input" :invalid="$form.name?.invalid" />
        <Message v-if="$form.name?.invalid" severity="error" size="small">{{ $form.name.error?.message }}</Message>
      </div>
      <div class="field">
        <label for="transport-kind">协议</label>
        <Select input-id="transport-kind" v-model="kind" :options="kindOptions" option-label="label" option-value="value" class="field-input" />
      </div>
      <Message v-if="kind === 'smb' && !smbEnabled" severity="warn">当前服务器未启用 SMB 转存，可保存配置，启用后方可使用。</Message>
      <div class="field">
        <label for="transport-host">服务器地址</label>
        <InputText id="transport-host" name="host" class="field-input" :invalid="$form.host?.invalid" />
        <Message v-if="$form.host?.invalid" severity="error" size="small">{{ $form.host.error?.message }}</Message>
      </div>
      <div v-if="kind === 'ftp'" class="field">
        <label for="transport-port">端口</label>
        <InputNumber input-id="transport-port" name="port" :min="1" :max="65535" :use-grouping="false" class="field-input" :invalid="$form.port?.invalid" />
        <Message v-if="$form.port?.invalid" severity="error" size="small">{{ $form.port.error?.message }}</Message>
      </div>
      <div v-if="kind === 'smb'" class="field">
        <label for="transport-share">共享名称</label>
        <InputText id="transport-share" name="share" class="field-input" :invalid="$form.share?.invalid" />
        <Message v-if="$form.share?.invalid" severity="error" size="small">{{ $form.share.error?.message }}</Message>
        <label for="transport-workgroup">工作组</label>
        <InputText id="transport-workgroup" name="workgroup" class="field-input" />
      </div>
      <div class="field"><label for="transport-user">用户名</label><InputText id="transport-user" name="username" autocomplete="off" class="field-input" /></div>
      <div class="field"><label for="transport-password">密码</label><Password input-id="transport-password" name="password" :feedback="false" toggle-mask autocomplete="new-password" :placeholder="editingId ? '留空保留原密码' : ''" class="field-input" /></div>
      <div class="field"><label for="transport-path">远端目录</label><InputText id="transport-path" name="base_path" class="field-input" /><small>目录下按设备 ID 分组保存录像。</small></div>
      <div class="transport-toolbar"><label for="transport-all">全部设备</label><ToggleSwitch input-id="transport-all" v-model="allDevices" /></div>
      <div v-if="!allDevices" class="field"><label for="transport-devices">选择设备</label><MultiSelect input-id="transport-devices" v-model="selectedDevices" :options="deviceOptions" option-label="label" option-value="value" filter class="field-input" /><small>未选择设备时不转存任何录像。</small></div>
      <div class="field"><label for="transport-remark">备注</label><InputText id="transport-remark" name="remark" class="field-input" /></div>
      <div class="transport-toolbar"><label for="transport-enabled">启用目标</label><ToggleSwitch input-id="transport-enabled" v-model="enabled" :disabled="kind === 'smb' && !smbEnabled" /></div>
      <div class="transport-toolbar"><Button label="取消" text :disabled="saving" @click="visible = false" /><Button type="submit" label="保存" :loading="saving" /></div>
    </Form>
  </Dialog>
</template>

<style scoped>
.transport-settings { margin-bottom: 1rem; }

.transport-title {
  color: #e2e8f0;
  font-size: 0.95rem;
  font-weight: 600;
}

.transport-icon { color: #38bdf8; }

.transport-hint {
  color: #94a3b8;
  font-size: 0.8rem;
  line-height: 1.5;
}

.transport-toolbar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.transport-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.transport-form .field {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}
</style>
