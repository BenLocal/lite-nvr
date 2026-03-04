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
.content-text {
  margin: 0;
  font-size: 0.875rem;
  color: var(--p-text-color);
  line-height: 1.6;
}
</style>
