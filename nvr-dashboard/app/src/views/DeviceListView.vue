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
.content-table {
  margin-top: 0.5rem;
}

.empty-message {
  padding: 2rem;
  text-align: center;
  color: var(--p-text-muted-color);
}
</style>
