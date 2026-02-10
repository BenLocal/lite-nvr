<script setup lang="ts">
import { ref } from 'vue'
import Card from 'primevue/card'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'

const devices = ref<{ id: string; name: string; status: string }[]>([])
</script>

<template>
  <div class="content-section">
    <Card class="content-card">
      <template #title>设备管理</template>
      <template #subtitle>管理接入的 NVR 设备列表。</template>
      <template #content>
        <DataTable :value="devices" striped-rows class="content-table" responsive-layout="scroll">
          <Column field="id" header="设备 ID" />
          <Column field="name" header="设备名称" />
          <Column field="status" header="状态" />
          <template #empty>
            <div class="empty-message">暂无设备数据</div>
          </template>
        </DataTable>
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

.content-table {
  margin-top: 0.5rem;
}

.empty-message {
  padding: 2rem;
  text-align: center;
  color: var(--p-text-muted-color);
}
</style>
