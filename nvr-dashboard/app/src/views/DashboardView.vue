<script setup lang="ts">
import Card from 'primevue/card'
</script>

<template>
  <div class="content-section">
    <Card class="content-card">
      <template #title>首页</template>
      <template #subtitle>欢迎使用 NVR 控制台。从左侧菜单进入设备管理。</template>
      <template #content>
        <p class="content-text">选择左侧菜单中的「设备管理」查看和管理接入的 NVR 设备。</p>
      </template>
    </Card>
  </div>
</template>

<style scoped>
.content-section {
  max-width: 100%;
}

.content-card {
  margin-bottom: 1rem;
  border: 1px solid var(--p-content-border-color, #e5e7eb);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
}

.content-card :deep(.p-card-title) {
  font-size: 1.125rem;
}

.content-card :deep(.p-card-subtitle) {
  font-size: 0.8125rem;
}

.content-card :deep(.p-card-content) {
  padding: 0.75rem 0;
}

.content-text {
  margin: 0;
  font-size: 0.875rem;
  color: var(--p-text-color);
  line-height: 1.6;
}
</style>
