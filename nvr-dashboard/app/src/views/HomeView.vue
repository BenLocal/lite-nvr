<script setup lang="ts">
import { useRouter } from 'vue-router'
import Button from 'primevue/button'
import { useAuth } from '../composables/useAuth'

const router = useRouter()
const { logout } = useAuth()

function onLogout() {
  logout()
  router.push('/login')
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
