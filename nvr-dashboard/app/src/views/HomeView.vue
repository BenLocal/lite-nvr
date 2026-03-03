<script setup lang="ts">
import { useRouter } from 'vue-router'
import Button from 'primevue/button'
import { useConfirm } from 'primevue/useconfirm'
import { useToast } from 'primevue/usetoast'
import { useAuth } from '../composables/useAuth'

const router = useRouter()
const { logout } = useAuth()
const confirm = useConfirm()
const toast = useToast()

function onLogout() {
  confirm.require({
    header: '退出登录',
    message: '确认退出当前账号吗？',
    icon: 'pi pi-exclamation-triangle',
    rejectLabel: '取消',
    acceptLabel: '确认',
    accept: async () => {
      logout()
      await router.push('/login')
      toast.add({
        severity: 'warn',
        summary: '已退出',
        detail: '你已安全退出登录',
        life: 2000,
      })
    },
    reject: () => {
      toast.add({
        severity: 'info',
        summary: '已取消',
        detail: '已取消退出操作',
        life: 1500,
      })
    },
  })
}
</script>

<template>
  <div class="home-page">
    <header class="home-header">
      <h1>NVR 控制台</h1>
      <Button label="退出登录" severity="secondary" @click="onLogout" />
    </header>
    <main class="home-main">
      <p>欢迎使用 NVR 控制台，登录已成功。</p>
    </main>
  </div>
</template>

<style scoped>
.home-page {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--p-surface-50);
}

.home-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.5rem;
  background: var(--p-surface-0);
  border-bottom: 1px solid var(--p-surface-200);
}

.home-header h1 {
  margin: 0;
  font-size: 1.25rem;
  font-weight: 600;
}

.home-main {
  flex: 1;
  padding: 1.5rem;
}
</style>
