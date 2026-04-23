<script setup lang="ts">
import Card from 'primevue/card'
import { ref } from 'vue'

const stats = ref([
  { label: '在线设备', value: '12', icon: 'pi-video', color: '#10b981', trend: '+2' },
  { label: '离线设备', value: '3', icon: 'pi-times-circle', color: '#ef4444', trend: '-1' },
  { label: '录像时长', value: '1.2TB', icon: 'pi-database', color: '#3b82f6', trend: '+120GB' },
  { label: '告警事件', value: '5', icon: 'pi-exclamation-triangle', color: '#f59e0b', trend: '+2' },
])

const recentDevices = ref([
  { name: '前门摄像头', status: 'online', location: '一楼大厅', fps: 25, bitrate: '2.5 Mbps' },
  { name: '后门摄像头', status: 'online', location: '一楼后门', fps: 25, bitrate: '2.3 Mbps' },
  { name: '停车场摄像头', status: 'offline', location: '地下停车场', fps: 0, bitrate: '0 Mbps' },
  { name: '会议室摄像头', status: 'online', location: '三楼会议室', fps: 30, bitrate: '3.1 Mbps' },
])
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
          <div class="stat-trend" :style="{ color: stat.color }">{{ stat.trend }}</div>
        </div>
      </div>
    </div>

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
          <div class="device-list">
            <div v-for="device in recentDevices" :key="device.name" class="device-item">
              <div class="device-info">
                <div class="device-status" :class="device.status">
                  <span class="status-dot"></span>
                </div>
                <div class="device-details">
                  <div class="device-name">{{ device.name }}</div>
                  <div class="device-location">{{ device.location }}</div>
                </div>
              </div>
              <div class="device-metrics">
                <div class="metric">
                  <span class="metric-label">FPS</span>
                  <span class="metric-value">{{ device.fps }}</span>
                </div>
                <div class="metric">
                  <span class="metric-label">码率</span>
                  <span class="metric-value">{{ device.bitrate }}</span>
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
