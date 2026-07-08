<script setup lang="ts">
import Card from 'primevue/card'
import ProgressBar from 'primevue/progressbar'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { getMetrics, getOverview, type SystemMetrics, type SystemOverview } from '../api/system'
import { useAppToast } from '../utils/toast'

const appToast = useAppToast()
const overview = ref<SystemOverview | null>(null)
const loading = ref(false)
const metrics = ref<SystemMetrics | null>(null)

function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB']
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1)
  const val = bytes / Math.pow(1024, i)
  return `${val.toFixed(i === 0 ? 0 : 1)} ${units[i]}`
}

function formatRate(bps: number): string {
  return `${formatBytes(bps)}/s`
}

const cpuPercent = computed(() => Math.min(100, Math.max(0, Math.round(metrics.value?.cpu_usage ?? 0))))
const memPercent = computed(() => {
  const m = metrics.value
  if (!m || m.mem_total <= 0) return 0
  return Math.min(100, Math.round((m.mem_used / m.mem_total) * 100))
})
const loadText = computed(() => {
  const m = metrics.value
  if (!m) return '— / — / —'
  return `${m.load_one.toFixed(2)} / ${m.load_five.toFixed(2)} / ${m.load_fifteen.toFixed(2)}`
})
const metricsUpdatedText = computed(() => {
  const ts = metrics.value?.sampled_at_ms ?? 0
  return ts > 0 ? new Date(ts).toLocaleTimeString('zh-CN', { hour12: false }) : '—'
})

const stats = computed(() => {
  const o = overview.value
  return [
    { label: '在线设备', value: String(o?.device_online ?? 0), icon: 'pi-video', color: '#10b981' },
    { label: '离线设备', value: String(o?.device_offline ?? 0), icon: 'pi-times-circle', color: '#ef4444' },
    { label: '录像存储', value: formatBytes(o?.record_total_bytes ?? 0), icon: 'pi-database', color: '#3b82f6' },
    { label: '录像片段', value: String(o?.record_segment_count ?? 0), icon: 'pi-file', color: '#f59e0b' },
  ]
})

const recentDevices = computed(() => (overview.value?.devices ?? []).slice(0, 6))

async function load() {
  loading.value = true
  try {
    overview.value = await getOverview()
  } catch (error) {
    appToast.errorFrom('加载失败', error, '系统概览加载失败')
  } finally {
    loading.value = false
  }
}

// Polled every few seconds. A transient failure just keeps the last snapshot —
// a background poll shouldn't spam error toasts.
async function loadMetrics() {
  try {
    metrics.value = await getMetrics()
  } catch {
    /* keep the previous sample */
  }
}

const METRICS_POLL_MS = 2000
let metricsTimer: number | undefined

onMounted(() => {
  void load()
  void loadMetrics()
  metricsTimer = window.setInterval(loadMetrics, METRICS_POLL_MS)
})

onUnmounted(() => {
  if (metricsTimer !== undefined) window.clearInterval(metricsTimer)
})
</script>

<template>
  <div class="dashboard">
    <div class="dashboard-header">
      <div class="header-content">
        <h1 class="dashboard-title">系统概览</h1>
        <p class="dashboard-subtitle">实时监控系统运行状态</p>
      </div>
      <div class="header-time">
        {{ new Date().toLocaleString('zh-CN', {
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
          hour: '2-digit',
          minute: '2-digit'
        }) }}
      </div>
    </div>

    <div class="stats-grid">
      <div v-for="stat in stats" :key="stat.label" class="stat-card">
        <div class="stat-icon" :style="{ background: `${stat.color}15`, color: stat.color }">
          <i :class="`pi ${stat.icon}`" />
        </div>
        <div class="stat-content">
          <div class="stat-label">{{ stat.label }}</div>
          <div class="stat-value">{{ stat.value }}</div>
        </div>
      </div>
    </div>

    <Card class="data-card perf-card">
      <template #header>
        <div class="card-header">
          <div class="card-header-left">
            <i class="pi pi-server card-header-icon" />
            <span class="card-header-title">系统性能</span>
          </div>
          <span class="perf-updated">
            <span class="perf-dot" />更新于 {{ metricsUpdatedText }}
          </span>
        </div>
      </template>
      <template #content>
        <div class="perf-grid">
          <div class="perf-item">
            <div class="perf-item-head">
              <span class="perf-label">CPU 使用率</span>
              <span class="perf-num">{{ cpuPercent }}%</span>
            </div>
            <ProgressBar :value="cpuPercent" :show-value="false" class="perf-bar" />
            <div class="perf-sub">{{ metrics?.cpu_core_count ?? 0 }} 核</div>
          </div>

          <div class="perf-item">
            <div class="perf-item-head">
              <span class="perf-label">内存</span>
              <span class="perf-num">{{ memPercent }}%</span>
            </div>
            <ProgressBar :value="memPercent" :show-value="false" class="perf-bar" />
            <div class="perf-sub">
              {{ formatBytes(metrics?.mem_used ?? 0) }} / {{ formatBytes(metrics?.mem_total ?? 0) }}
            </div>
          </div>

          <div class="perf-item">
            <div class="perf-item-head">
              <span class="perf-label">网络</span>
            </div>
            <div class="perf-net">
              <span class="net-rate net-down"><i class="pi pi-arrow-down" />{{ formatRate(metrics?.net_rx_bps ?? 0) }}</span>
              <span class="net-rate net-up"><i class="pi pi-arrow-up" />{{ formatRate(metrics?.net_tx_bps ?? 0) }}</span>
            </div>
            <div class="perf-sub">
              总 ↓{{ formatBytes(metrics?.net_rx_total ?? 0) }} · ↑{{ formatBytes(metrics?.net_tx_total ?? 0) }}
            </div>
          </div>

          <div class="perf-item">
            <div class="perf-item-head">
              <span class="perf-label">系统负载</span>
            </div>
            <div class="perf-load">{{ loadText }}</div>
            <div class="perf-sub">1 / 5 / 15 分钟</div>
          </div>
        </div>
      </template>
    </Card>

    <div class="content-grid">
      <Card class="data-card">
        <template #header>
          <div class="card-header">
            <div class="card-header-left">
              <i class="pi pi-video card-header-icon" />
              <span class="card-header-title">设备状态</span>
            </div>
            <router-link to="/device" class="card-header-link">
              查看全部 <i class="pi pi-arrow-right" />
            </router-link>
          </div>
        </template>
        <template #content>
          <div v-if="!loading && !recentDevices.length" class="device-empty">暂无设备</div>
          <div v-else class="device-list">
            <div v-for="device in recentDevices" :key="device.id" class="device-item">
              <div class="device-info">
                <div class="device-status" :class="device.online ? 'online' : 'offline'">
                  <span class="status-dot"></span>
                </div>
                <div class="device-details">
                  <div class="device-name">{{ device.name }}</div>
                  <div class="device-location">{{ device.description || device.input_type }}</div>
                </div>
              </div>
              <div class="device-metrics">
                <div class="metric">
                  <span class="metric-label">类型</span>
                  <span class="metric-value">{{ device.input_type }}</span>
                </div>
                <div class="metric">
                  <span class="metric-label">录制</span>
                  <span class="metric-value">{{ device.record ? '开' : '关' }}</span>
                </div>
              </div>
            </div>
          </div>
        </template>
      </Card>

      <Card class="data-card">
        <template #header>
          <div class="card-header">
            <div class="card-header-left">
              <i class="pi pi-clock card-header-icon" />
              <span class="card-header-title">最近活动</span>
            </div>
          </div>
        </template>
        <template #content>
          <div class="activity-list">
            <div class="activity-item">
              <div class="activity-time">10:23</div>
              <div class="activity-content">
                <div class="activity-title">设备上线</div>
                <div class="activity-desc">前门摄像头重新连接</div>
              </div>
            </div>
            <div class="activity-item">
              <div class="activity-time">09:45</div>
              <div class="activity-content">
                <div class="activity-title">录像完成</div>
                <div class="activity-desc">停车场摄像头录像已保存</div>
              </div>
            </div>
            <div class="activity-item">
              <div class="activity-time">08:12</div>
              <div class="activity-content">
                <div class="activity-title">系统启动</div>
                <div class="activity-desc">NVR 系统正常启动</div>
              </div>
            </div>
          </div>
        </template>
      </Card>
    </div>
  </div>
</template>

<style scoped>
.dashboard {
  animation: fade-in 0.4s ease-out;
}

@keyframes fade-in {
  from {
    opacity: 0;
    transform: translateY(10px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.dashboard-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 1.5rem;
  padding-bottom: 1rem;
  border-bottom: 1px solid rgb(148 163 184 / 10%);
}

.header-content {
  flex: 1;
}

.dashboard-title {
  font-size: 1.25rem;
  font-weight: 600;
  color: #e2e8f0;
  margin: 0 0 0.25rem;
  letter-spacing: -0.025em;
}

.dashboard-subtitle {
  font-size: 0.8125rem;
  color: #94a3b8;
  margin: 0;
}

.header-time {
  font-size: 0.75rem;
  color: #64748b;
  font-variant-numeric: tabular-nums;
  padding: 0.375rem 0.75rem;
  background: rgb(148 163 184 / 5%);
  border: 1px solid rgb(148 163 184 / 10%);
  border-radius: 0.375rem;
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.stat-card {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1.25rem;
  background: rgb(15 23 42 / 40%);
  backdrop-filter: blur(12px);
  border: 1px solid rgb(148 163 184 / 10%);
  border-radius: 0.75rem;
  transition: all 0.3s;
  animation: slide-up 0.5s ease-out backwards;
}

.stat-card:nth-child(1) { animation-delay: 0.05s; }
.stat-card:nth-child(2) { animation-delay: 0.1s; }
.stat-card:nth-child(3) { animation-delay: 0.15s; }
.stat-card:nth-child(4) { animation-delay: 0.2s; }

@keyframes slide-up {
  from {
    opacity: 0;
    transform: translateY(20px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.stat-card:hover {
  transform: translateY(-2px);
  border-color: rgb(148 163 184 / 20%);
  box-shadow: 0 8px 24px rgb(0 0 0 / 30%);
}

.stat-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 3rem;
  height: 3rem;
  border-radius: 0.75rem;
  font-size: 1.5rem;
  flex-shrink: 0;
}

.stat-content {
  flex: 1;
  min-width: 0;
}

.stat-label {
  font-size: 0.75rem;
  color: #94a3b8;
  margin-bottom: 0.25rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.stat-value {
  font-size: 1.75rem;
  font-weight: 700;
  color: #e2e8f0;
  line-height: 1;
  margin-bottom: 0.25rem;
  font-variant-numeric: tabular-nums;
}

.stat-trend {
  font-size: 0.75rem;
  font-weight: 600;
}

.content-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: 1.5rem;
}

.data-card {
  animation: slide-up 0.5s ease-out 0.25s backwards;
}

.perf-card {
  margin-bottom: 1.5rem;
  animation: slide-up 0.5s ease-out 0.22s backwards;
}

.perf-updated {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.6875rem;
  color: #64748b;
  font-variant-numeric: tabular-nums;
}

.perf-dot {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: #10b981;
  box-shadow: 0 0 8px rgb(16 185 129 / 60%);
  animation: pulse 2s ease-in-out infinite;
}

.perf-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 1rem;
}

.perf-item {
  padding: 1rem;
  background: rgb(30 41 59 / 40%);
  border: 1px solid rgb(148 163 184 / 8%);
  border-radius: 0.5rem;
}

.perf-item-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 0.625rem;
}

.perf-label {
  font-size: 0.6875rem;
  color: #94a3b8;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.perf-num {
  font-size: 1.25rem;
  font-weight: 700;
  color: #e2e8f0;
  line-height: 1;
  font-variant-numeric: tabular-nums;
}

.perf-bar {
  margin-bottom: 0.5rem;
}

.perf-bar :deep(.p-progressbar) {
  height: 0.5rem;
  background: rgb(148 163 184 / 12%);
}

.perf-sub {
  font-size: 0.6875rem;
  color: #64748b;
  font-variant-numeric: tabular-nums;
}

.perf-net {
  display: flex;
  gap: 1rem;
  margin-bottom: 0.5rem;
}

.net-rate {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: 0.9375rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.net-rate i {
  font-size: 0.75rem;
}

.net-down {
  color: #38bdf8;
}

.net-up {
  color: #f59e0b;
}

.perf-load {
  font-size: 1.125rem;
  font-weight: 700;
  color: #e2e8f0;
  font-variant-numeric: tabular-nums;
  margin-bottom: 0.5rem;
}

/* Card header styles */
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-bottom: 1rem;
  border-bottom: 1px solid rgb(148 163 184 / 10%);
}

.card-header-left {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.card-header-icon {
  font-size: 1rem;
  color: #3b82f6;
}

.card-header-title {
  font-size: 0.875rem;
  font-weight: 600;
  color: #e2e8f0;
}

.card-header-link {
  font-size: 0.75rem;
  color: #3b82f6;
  text-decoration: none;
  display: flex;
  align-items: center;
  gap: 0.25rem;
  transition: all 0.2s;
}

.card-header-link:hover {
  color: #60a5fa;
  gap: 0.5rem;
}

.device-list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.device-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.875rem;
  background: rgb(30 41 59 / 40%);
  border: 1px solid rgb(148 163 184 / 8%);
  border-radius: 0.5rem;
  transition: all 0.2s;
}

.device-item:hover {
  background: rgb(30 41 59 / 60%);
  border-color: rgb(148 163 184 / 15%);
}

.device-info {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  flex: 1;
  min-width: 0;
}

.device-status {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  border-radius: 0.5rem;
  background: rgb(148 163 184 / 10%);
  flex-shrink: 0;
}

.device-status.online {
  background: rgb(16 185 129 / 15%);
}

.device-status.offline {
  background: rgb(239 68 68 / 15%);
}

.status-dot {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: #64748b;
}

.device-status.online .status-dot {
  background: #10b981;
  box-shadow: 0 0 8px rgb(16 185 129 / 60%);
  animation: pulse 2s ease-in-out infinite;
}

.device-status.offline .status-dot {
  background: #ef4444;
}

.device-details {
  flex: 1;
  min-width: 0;
}

.device-name {
  font-size: 0.8125rem;
  font-weight: 500;
  color: #e2e8f0;
  margin-bottom: 0.125rem;
}

.device-location {
  font-size: 0.6875rem;
  color: #64748b;
}

.device-empty {
  padding: 2rem;
  text-align: center;
  font-size: 0.8125rem;
  color: #94a3b8;
}

.device-metrics {
  display: flex;
  gap: 1rem;
  flex-shrink: 0;
}

.metric {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 0.125rem;
}

.metric-label {
  font-size: 0.625rem;
  color: #64748b;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.metric-value {
  font-size: 0.75rem;
  font-weight: 600;
  color: #94a3b8;
  font-variant-numeric: tabular-nums;
}

.activity-list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.activity-item {
  display: flex;
  gap: 1rem;
  padding: 0.875rem;
  background: rgb(30 41 59 / 40%);
  border: 1px solid rgb(148 163 184 / 8%);
  border-radius: 0.5rem;
  transition: all 0.2s;
}

.activity-item:hover {
  background: rgb(30 41 59 / 60%);
  border-color: rgb(148 163 184 / 15%);
}

.activity-time {
  font-size: 0.6875rem;
  color: #64748b;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
  padding-top: 0.125rem;
}

.activity-content {
  flex: 1;
  min-width: 0;
}

.activity-title {
  font-size: 0.8125rem;
  font-weight: 500;
  color: #e2e8f0;
  margin-bottom: 0.125rem;
}

.activity-desc {
  font-size: 0.6875rem;
  color: #64748b;
}

@media (width <= 768px) {
  .stats-grid {
    grid-template-columns: 1fr;
  }

  .content-grid {
    grid-template-columns: 1fr;
  }

  .device-metrics {
    flex-direction: column;
    gap: 0.5rem;
  }
}
</style>
