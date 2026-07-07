<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Card from 'primevue/card'
import Select from 'primevue/select'
import Button from 'primevue/button'
import { useAppToast } from '../utils/toast'
import type { PlayerBackend } from '../api/settings'
import {
  ensurePlayerPreference,
  savePlayerBackend,
  usePlayerPreference,
} from '../composables/usePlayerPreference'

const appToast = useAppToast()
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

onMounted(async () => {
  await ensurePlayerPreference()
  player.value = playerBackend.value
  loading.value = false
})

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
  </div>
</template>

<style scoped>
.settings-card {
  max-width: 44rem;
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

.settings-hint {
  margin: 0.5rem 0 0;
  font-size: 0.8rem;
  line-height: 1.5;
  color: #94a3b8;
}

.settings-actions {
  margin-top: 1.25rem;
}
</style>
