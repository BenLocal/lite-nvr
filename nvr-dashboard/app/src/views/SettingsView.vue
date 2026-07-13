<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Card from 'primevue/card'
import Column from 'primevue/column'
import DataTable from 'primevue/datatable'
import Dialog from 'primevue/dialog'
import Password from 'primevue/password'
import InputText from 'primevue/inputtext'
import Select from 'primevue/select'
import Button from 'primevue/button'
import InputNumber from 'primevue/inputnumber'
import ToggleSwitch from 'primevue/toggleswitch'
import { useConfirm } from 'primevue/useconfirm'
import { useAppToast } from '../utils/toast'
import { getCleanup, saveCleanup, type CleanupConfig, type PlayerBackend } from '../api/settings'
import {
  addUser,
  changePassword,
  getUserInfo,
  listUsers,
  removeUser,
  type UserListItem,
} from '../api/user'
import {
  ensurePlayerPreference,
  savePlayerBackend,
  usePlayerPreference,
} from '../composables/usePlayerPreference'

const appToast = useAppToast()
const confirm = useConfirm()
const { playerBackend } = usePlayerPreference()

const player = ref<PlayerBackend>('mpegts')
const saving = ref(false)
const loading = ref(true)

const playerOptions: { value: PlayerBackend; label: string; hint: string }[] = [
  {
    value: 'mpegts',
    label: 'mpegts.js（默认）',
    hint: '走浏览器 MSE 原生（硬件）解码，CPU 占用低，多画面导播台推荐。H.265 需浏览器支持硬解。',
  },
  {
    value: 'jessibuca',
    label: 'Jessibuca（WASM 软解）',
    hint: 'WebAssembly 软件解码，兼容性最好（含浏览器不支持硬解的 H.265），但多路会明显吃 CPU。',
  },
  {
    value: 'auto',
    label: '自动',
    hint: '优先用 mpegts.js（硬解）；播放失败时自动回退到 Jessibuca（软解）兜底。',
  },
]

const currentHint = computed(
  () => playerOptions.find((o) => o.value === player.value)?.hint ?? '',
)

async function onSave() {
  saving.value = true
  try {
    await savePlayerBackend(player.value)
    appToast.success('已保存', '播放器设置已更新')
  } catch (error) {
    appToast.errorFrom('保存失败', error, '无法保存设置')
  } finally {
    saving.value = false
  }
}

// ---- Record-segment cleanup ----------------------------------------------
const cleanup = ref<CleanupConfig>({
  enabled: false,
  max_age_days: 0,
  max_total_gb: 0,
  interval_minutes: 60,
})
const cleanupLoading = ref(true)
const cleanupSaving = ref(false)

async function loadCleanup() {
  try {
    cleanup.value = await getCleanup()
  } catch {
    // keep defaults if the endpoint isn't available yet (pre-rebuild)
  } finally {
    cleanupLoading.value = false
  }
}

async function onSaveCleanup() {
  cleanupSaving.value = true
  try {
    cleanup.value = await saveCleanup(cleanup.value)
    appToast.success('已保存', '录制清理策略已更新')
  } catch (error) {
    appToast.errorFrom('保存失败', error, '无法保存清理策略')
  } finally {
    cleanupSaving.value = false
  }
}

// ---- Account & security ---------------------------------------------------
const currentUsername = ref('')
const oldPassword = ref('')
const newPassword = ref('')
const confirmNewPassword = ref('')
const passwordSaving = ref(false)

const users = ref<UserListItem[]>([])
const usersLoading = ref(true)
const addUserVisible = ref(false)
const addUsername = ref('')
const addUserPassword = ref('')
const addUserSaving = ref(false)

async function loadAccount() {
  try {
    currentUsername.value = (await getUserInfo()).username
  } catch {
    // an invalid session is handled globally by the request wrapper
  }
}

async function loadUsers() {
  usersLoading.value = true
  try {
    users.value = await listUsers()
  } catch (error) {
    appToast.errorFrom('加载失败', error, '无法获取用户列表')
  } finally {
    usersLoading.value = false
  }
}

async function onChangePassword() {
  if (!oldPassword.value || !newPassword.value) {
    appToast.warn('请完整填写', '旧密码和新密码都不能为空')
    return
  }
  if (newPassword.value !== confirmNewPassword.value) {
    appToast.warn('两次输入不一致', '请重新确认新密码')
    return
  }
  passwordSaving.value = true
  try {
    await changePassword({
      old_password: oldPassword.value,
      new_password: newPassword.value,
    })
    oldPassword.value = ''
    newPassword.value = ''
    confirmNewPassword.value = ''
    appToast.success('已修改', '密码已更新，该账号的其他登录会话已失效')
  } catch (error) {
    appToast.errorFrom('修改失败', error, '无法修改密码')
  } finally {
    passwordSaving.value = false
  }
}

function openAddUser() {
  addUsername.value = ''
  addUserPassword.value = ''
  addUserVisible.value = true
}

async function onAddUser() {
  const username = addUsername.value.trim()
  if (!username || !addUserPassword.value) {
    appToast.warn('请完整填写', '用户名和密码都不能为空')
    return
  }
  addUserSaving.value = true
  try {
    await addUser({ username, password: addUserPassword.value })
    addUserVisible.value = false
    appToast.success('已创建', `用户 ${username} 已创建`)
    await loadUsers()
  } catch (error) {
    appToast.errorFrom('创建失败', error, '无法创建用户')
  } finally {
    addUserSaving.value = false
  }
}

function onRemoveUser(user: UserListItem) {
  confirm.require({
    header: '删除用户',
    message: `确认删除用户「${user.username}」吗？其所有登录会话将立即失效。`,
    icon: 'pi pi-exclamation-triangle',
    rejectLabel: '取消',
    acceptLabel: '删除',
    accept: async () => {
      try {
        await removeUser(user.username)
        appToast.success('已删除', `用户 ${user.username} 已删除`)
        await loadUsers()
      } catch (error) {
        appToast.errorFrom('删除失败', error, '无法删除用户')
      }
    },
  })
}

function formatUserTime(iso: string) {
  const date = new Date(iso)
  return Number.isNaN(date.getTime()) ? '-' : date.toLocaleString()
}

onMounted(async () => {
  await ensurePlayerPreference()
  player.value = playerBackend.value
  loading.value = false
  await Promise.all([loadCleanup(), loadAccount(), loadUsers()])
})
</script>

<template>
  <div class="content-section">
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">设置</h1>
        <p class="page-subtitle">配置控制台的播放与显示行为</p>
      </div>
    </div>

    <Card class="data-card settings-card">
      <template #header>
        <div class="settings-card-header">
          <i class="pi pi-play-circle settings-card-icon" />
          <span class="settings-card-title">播放器</span>
        </div>
      </template>
      <template #content>
        <div class="field settings-field">
          <label for="player">播放方式</label>
          <Select
            id="player"
            v-model="player"
            :options="playerOptions"
            option-label="label"
            option-value="value"
            size="small"
            class="field-input"
            :disabled="loading"
          />
          <p class="settings-hint">{{ currentHint }}</p>
        </div>

        <div class="settings-actions">
          <Button
            label="保存"
            icon="pi pi-check"
            size="small"
            :loading="saving"
            :disabled="loading"
            @click="onSave"
          />
        </div>
      </template>
    </Card>

    <Card class="data-card settings-card">
      <template #header>
        <div class="settings-card-header">
          <i class="pi pi-trash settings-card-icon" />
          <span class="settings-card-title">录制清理</span>
        </div>
      </template>
      <template #content>
        <div class="cleanup-toggle">
          <label for="cleanup-enabled">启用定时清理</label>
          <ToggleSwitch
            input-id="cleanup-enabled"
            v-model="cleanup.enabled"
            :disabled="cleanupLoading"
          />
        </div>

        <div class="field settings-field">
          <label for="cleanup-age">保留天数</label>
          <InputNumber
            input-id="cleanup-age"
            v-model="cleanup.max_age_days"
            :min="0"
            :max="3650"
            suffix=" 天"
            show-buttons
            size="small"
            class="field-input"
            :disabled="cleanupLoading || !cleanup.enabled"
          />
          <p class="settings-hint">删除超过该天数的录制片段;0 表示不按时间清理。</p>
        </div>

        <div class="field settings-field">
          <label for="cleanup-size">总容量上限</label>
          <InputNumber
            input-id="cleanup-size"
            v-model="cleanup.max_total_gb"
            :min="0"
            :max="1048576"
            suffix=" GB"
            show-buttons
            size="small"
            class="field-input"
            :disabled="cleanupLoading || !cleanup.enabled"
          />
          <p class="settings-hint">录制总大小超过该值时,从最旧的片段开始删除;0 表示不按容量清理。</p>
        </div>

        <div class="field settings-field">
          <label for="cleanup-interval">运行间隔</label>
          <InputNumber
            input-id="cleanup-interval"
            v-model="cleanup.interval_minutes"
            :min="1"
            :max="10080"
            suffix=" 分钟"
            show-buttons
            size="small"
            class="field-input"
            :disabled="cleanupLoading || !cleanup.enabled"
          />
          <p class="settings-hint">后台清理任务的执行周期。</p>
        </div>

        <div class="settings-actions">
          <Button
            label="保存"
            icon="pi pi-check"
            size="small"
            :loading="cleanupSaving"
            :disabled="cleanupLoading"
            @click="onSaveCleanup"
          />
        </div>
      </template>
    </Card>

    <Card class="data-card settings-card">
      <template #header>
        <div class="settings-card-header">
          <i class="pi pi-shield settings-card-icon" />
          <span class="settings-card-title">账户安全</span>
        </div>
      </template>
      <template #content>
        <p class="settings-hint account-current">
          当前账号：<span class="mono-text">{{ currentUsername || '-' }}</span>
        </p>

        <div class="field settings-field">
          <label for="old-password">旧密码</label>
          <Password
            input-id="old-password"
            v-model="oldPassword"
            :feedback="false"
            toggle-mask
            size="small"
            class="field-input"
          />
        </div>

        <div class="field settings-field">
          <label for="new-password">新密码</label>
          <Password
            input-id="new-password"
            v-model="newPassword"
            :feedback="false"
            toggle-mask
            size="small"
            class="field-input"
          />
        </div>

        <div class="field settings-field">
          <label for="confirm-password">确认新密码</label>
          <Password
            input-id="confirm-password"
            v-model="confirmNewPassword"
            :feedback="false"
            toggle-mask
            size="small"
            class="field-input"
          />
          <p class="settings-hint">修改成功后，该账号在其他设备上的登录会话将全部失效。</p>
        </div>

        <div class="settings-actions">
          <Button
            label="修改密码"
            icon="pi pi-key"
            size="small"
            :loading="passwordSaving"
            @click="onChangePassword"
          />
        </div>
      </template>
    </Card>

    <Card class="data-card settings-card">
      <template #header>
        <div class="settings-card-header">
          <i class="pi pi-users settings-card-icon" />
          <span class="settings-card-title">用户管理</span>
          <Button
            label="添加用户"
            icon="pi pi-plus"
            size="small"
            class="user-add-button"
            @click="openAddUser"
          />
        </div>
      </template>
      <template #content>
        <DataTable :value="users" :loading="usersLoading" size="small">
          <template #empty>
            <div class="empty-state">
              <i class="pi pi-users empty-state-icon" />
              <p class="empty-state-text">暂无用户</p>
            </div>
          </template>
          <Column field="username" header="用户名">
            <template #body="{ data }">
              <span class="mono-text">{{ data.username }}</span>
              <span v-if="data.username === currentUsername" class="user-self-tag">当前</span>
            </template>
          </Column>
          <Column field="create_time" header="创建时间">
            <template #body="{ data }">{{ formatUserTime(data.create_time) }}</template>
          </Column>
          <Column field="update_time" header="更新时间">
            <template #body="{ data }">{{ formatUserTime(data.update_time) }}</template>
          </Column>
          <Column header="操作" class="user-actions-column">
            <template #body="{ data }">
              <Button
                icon="pi pi-trash"
                severity="danger"
                text
                size="small"
                :disabled="data.username === currentUsername"
                @click="onRemoveUser(data)"
              />
            </template>
          </Column>
        </DataTable>
      </template>
    </Card>

    <Dialog
      v-model:visible="addUserVisible"
      header="添加用户"
      modal
      class="user-add-dialog"
    >
      <div class="field settings-field">
        <label for="add-username">用户名</label>
        <InputText
          id="add-username"
          v-model="addUsername"
          size="small"
          class="field-input"
          autocomplete="off"
        />
      </div>
      <div class="field settings-field">
        <label for="add-password">密码</label>
        <Password
          input-id="add-password"
          v-model="addUserPassword"
          :feedback="false"
          toggle-mask
          size="small"
          class="field-input"
        />
      </div>
      <template #footer>
        <Button
          label="取消"
          severity="secondary"
          text
          size="small"
          @click="addUserVisible = false"
        />
        <Button
          label="创建"
          icon="pi pi-check"
          size="small"
          :loading="addUserSaving"
          @click="onAddUser"
        />
      </template>
    </Dialog>
  </div>
</template>

<style scoped>
.settings-card {
  max-width: 44rem;
}

/* keep 10px between stacked setting cards (global .data-card has no margin) */
.settings-card + .settings-card {
  margin-top: 10px;
}

.settings-card-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 1rem 1.25rem 0;
}

.settings-card-icon {
  color: #38bdf8;
}

.settings-card-title {
  font-size: 0.95rem;
  font-weight: 600;
  color: #e2e8f0;
}

.settings-field {
  max-width: 32rem;
}

.cleanup-toggle {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 1.25rem;
}

.settings-hint {
  margin: 0.5rem 0 0;
  font-size: 0.8rem;
  line-height: 1.5;
  color: #94a3b8;
}

.settings-actions {
  margin-top: 1.25rem;
}

.account-current {
  margin: 0 0 1.25rem;
}

.user-add-button {
  margin-left: auto;
}

.user-self-tag {
  margin-left: 0.5rem;
  padding: 0.1rem 0.4rem;
  border: 1px solid rgb(59 130 246 / 40%);
  border-radius: 0.375rem;
  font-size: 0.7rem;
  color: #38bdf8;
}

.user-add-dialog {
  width: 22rem;
}
</style>
