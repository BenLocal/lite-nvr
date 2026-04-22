<script setup lang="ts">
import { ref, computed } from 'vue'
import { useRouter } from 'vue-router'
import Menu from 'primevue/menu'
import Button from 'primevue/button'
import Avatar from 'primevue/avatar'
import { useConfirm } from 'primevue/useconfirm'
import { useToast } from 'primevue/usetoast'
import { useAuth } from '../composables/useAuth'

const router = useRouter()
const { logout } = useAuth()
const confirm = useConfirm()
const toast = useToast()

const sidebarCollapsed = ref(false)
const userMenu = ref<InstanceType<typeof Menu> | null>(null)
const baseUrl = import.meta.env.BASE_URL
const logoUrl = ref(`${baseUrl}logo.svg`)

const menuItems = ref([
  { label: '概览', route: '/', icon: 'pi pi-th-large' },
  { label: '设备', route: '/device', icon: 'pi pi-video' },
  { label: '回放', route: '/playback', icon: 'pi pi-play-circle' },
])

const userMenuItems = ref([
  {
    label: '退出登录',
    icon: 'pi pi-sign-out',
    command: onLogoutRequest,
  },
])

const mainContainerClass = computed(() => [
  'layout-main-container',
  { 'layout-sidebar-inactive': sidebarCollapsed.value },
])

const sidebarClass = computed(() => [
  'layout-sidebar',
  { 'layout-sidebar-collapsed': sidebarCollapsed.value },
])

function toggleSidebar() {
  sidebarCollapsed.value = !sidebarCollapsed.value
}

function toggleUserMenu(event: Event) {
  userMenu.value?.toggle(event)
}

function onLogoError() {
  logoUrl.value = ''
}

function onLogoutRequest() {
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
  <div class="layout-wrapper">
    <header class="layout-topbar">
      <div class="topbar-left">
        <Button
          type="button"
          icon="pi pi-bars"
          text
          class="topbar-menu-button"
          aria-label="切换侧边栏"
          @click="toggleSidebar"
        />
        <router-link to="/" class="topbar-brand">
          <div class="topbar-logo">
            <i class="pi pi-video" />
          </div>
          <span class="topbar-title">NVR Console</span>
        </router-link>
      </div>

      <div class="topbar-right">
        <div class="topbar-status">
          <span class="status-indicator status-online"></span>
          <span class="status-text">系统运行中</span>
        </div>
        <Button
          type="button"
          icon="pi pi-bell"
          text
          severity="secondary"
          class="topbar-icon-button"
          aria-label="通知"
        />
        <Button
          type="button"
          text
          severity="secondary"
          class="topbar-user-button"
          aria-label="用户菜单"
          @click="toggleUserMenu"
        >
          <Avatar icon="pi pi-user" shape="circle" size="small" />
        </Button>
        <Menu ref="userMenu" :model="userMenuItems" :popup="true" />
      </div>
    </header>

    <aside :class="sidebarClass">
      <nav class="layout-menu">
        <ul class="layout-menu-list">
          <li v-for="item in menuItems" :key="item.route" class="layout-menuitem">
            <router-link
              :to="item.route"
              class="layout-menuitem-link"
              :class="{ 'active-route': $route.path === item.route || $route.path.startsWith(`${item.route}/`) }"
            >
              <i :class="['layout-menuitem-icon', item.icon]" />
              <span class="layout-menuitem-text">{{ item.label }}</span>
            </router-link>
          </li>
        </ul>
      </nav>
    </aside>

    <div :class="mainContainerClass">
      <main class="layout-main">
        <RouterView />
      </main>
    </div>
  </div>
</template>

<style scoped>
.layout-wrapper {
  --layout-sidebar-width: 4rem;
  --layout-topbar-height: 3.5rem;
  --layout-gap: 0;

  min-height: 100vh;
  background: linear-gradient(135deg, #0f172a 0%, #1e293b 100%);
  font-family: -apple-system, BlinkMacSystemFont, 'Inter', 'SF Pro Display', sans-serif;
}

/* Topbar */
.layout-topbar {
  position: fixed;
  left: 0;
  top: 0;
  width: 100%;
  height: var(--layout-topbar-height);
  padding: 0 1.25rem;
  z-index: 997;
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: rgba(15, 23, 42, 0.8);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid rgba(148, 163, 184, 0.1);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
}

.topbar-left {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.topbar-menu-button {
  width: 2rem;
  height: 2rem;
  color: #94a3b8;
  transition: all 0.2s;
}

.topbar-menu-button:hover {
  color: #e2e8f0;
  background: rgba(148, 163, 184, 0.1);
}

.topbar-brand {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  text-decoration: none;
  color: #e2e8f0;
  transition: opacity 0.2s;
}

.topbar-brand:hover {
  opacity: 0.8;
}

.topbar-logo {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  border-radius: 0.5rem;
  background: linear-gradient(135deg, #3b82f6 0%, #2563eb 100%);
  box-shadow: 0 2px 8px rgba(59, 130, 246, 0.3);
  font-size: 1rem;
  color: white;
}

.topbar-title {
  font-size: 0.875rem;
  font-weight: 600;
  letter-spacing: 0.025em;
  color: #e2e8f0;
}

.topbar-right {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.topbar-status {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem 0.75rem;
  background: rgba(16, 185, 129, 0.1);
  border: 1px solid rgba(16, 185, 129, 0.2);
  border-radius: 0.375rem;
  margin-right: 0.5rem;
}

.status-indicator {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  animation: pulse 2s ease-in-out infinite;
}

.status-online {
  background: #10b981;
  box-shadow: 0 0 8px rgba(16, 185, 129, 0.6);
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.5;
  }
}

.status-text {
  font-size: 0.75rem;
  color: #10b981;
  font-weight: 500;
}

.topbar-icon-button {
  width: 2rem;
  height: 2rem;
  color: #94a3b8;
  transition: all 0.2s;
}

.topbar-icon-button:hover {
  color: #e2e8f0;
  background: rgba(148, 163, 184, 0.1);
}

.topbar-user-button {
  padding: 0.25rem;
  transition: all 0.2s;
}

.topbar-user-button:hover {
  background: rgba(148, 163, 184, 0.1);
}

/* Sidebar */
.layout-sidebar {
  position: fixed;
  width: var(--layout-sidebar-width);
  left: 0;
  top: var(--layout-topbar-height);
  height: calc(100vh - var(--layout-topbar-height));
  z-index: 996;
  background: rgba(15, 23, 42, 0.6);
  backdrop-filter: blur(12px);
  border-right: 1px solid rgba(148, 163, 184, 0.1);
  transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  overflow: hidden;
}

.layout-sidebar-collapsed {
  transform: translateX(-100%);
}

.layout-menu {
  padding: 0.5rem 0;
}

.layout-menu-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.layout-menuitem {
  margin: 0;
}

.layout-menuitem-link {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.25rem;
  padding: 0.875rem 0.5rem;
  color: #94a3b8;
  text-decoration: none;
  transition: all 0.2s;
  cursor: pointer;
  position: relative;
  border-left: 3px solid transparent;
}

.layout-menuitem-link:hover {
  background: rgba(59, 130, 246, 0.1);
  color: #60a5fa;
  border-left-color: rgba(59, 130, 246, 0.3);
}

.layout-menuitem-link.active-route {
  color: #3b82f6;
  background: rgba(59, 130, 246, 0.15);
  border-left-color: #3b82f6;
}

.layout-menuitem-link.active-route::before {
  content: '';
  position: absolute;
  left: 0;
  top: 50%;
  transform: translateY(-50%);
  width: 3px;
  height: 60%;
  background: linear-gradient(180deg, transparent, #3b82f6, transparent);
  box-shadow: 0 0 8px rgba(59, 130, 246, 0.6);
}

.layout-menuitem-icon {
  font-size: 1.25rem;
  flex-shrink: 0;
}

.layout-menuitem-text {
  font-size: 0.625rem;
  font-weight: 500;
  text-align: center;
  letter-spacing: 0.025em;
  text-transform: uppercase;
}

/* Main content */
.layout-main-container {
  margin-left: var(--layout-sidebar-width);
  margin-top: var(--layout-topbar-height);
  min-height: calc(100vh - var(--layout-topbar-height));
  padding: 1.5rem;
  transition: margin-left 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.layout-main-container.layout-sidebar-inactive {
  margin-left: 0;
}

.layout-main {
  max-width: 1400px;
  margin: 0 auto;
}

/* Responsive */
@media (width <= 768px) {
  .layout-sidebar {
    width: 16rem;
    box-shadow: 2px 0 12px rgba(0, 0, 0, 0.5);
  }

  .layout-menuitem-link {
    flex-direction: row;
    justify-content: flex-start;
    padding: 0.75rem 1rem;
    gap: 0.75rem;
  }

  .layout-menuitem-text {
    font-size: 0.875rem;
    text-transform: none;
  }

  .layout-main-container {
    margin-left: 0;
  }

  .topbar-status {
    display: none;
  }
}
</style>
