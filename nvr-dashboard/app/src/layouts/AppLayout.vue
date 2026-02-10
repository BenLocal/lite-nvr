<script setup lang="ts">
import { ref, computed } from 'vue'
import { useRouter } from 'vue-router'
import Menu from 'primevue/menu'
import Button from 'primevue/button'
import { useAuth } from '../composables/useAuth'

const router = useRouter()
const { logout } = useAuth()

const sidebarCollapsed = ref(false)
const userMenu = ref<InstanceType<typeof Menu> | null>(null)
const baseUrl = import.meta.env.BASE_URL
const logoUrl = ref(`${baseUrl}logo.svg`)

const menuItems = ref([
  { label: '首页', route: '/', icon: 'pi pi-fw pi-home' },
  { label: '设备管理', route: '/devices', icon: 'pi pi-fw pi-video' },
])

const userMenuItems = ref([
  {
    label: '退出登录',
    icon: 'pi pi-sign-out',
    command: () => {
      logout()
      router.push('/login')
    },
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
</script>

<template>
  <div class="layout-wrapper">
    <header class="layout-topbar">
      <div class="topbar-left">
        <Button
            type="button"
            icon="pi pi-bars"
            text
            rounded
            class="topbar-menu-button"
            aria-label="切换侧边栏"
            @click="toggleSidebar"
          />
        <router-link to="/" class="topbar-brand">
          <img
            v-if="logoUrl"
            :src="logoUrl"
            alt="NVR"
            class="topbar-logo-img"
            @error="onLogoError"
          />
          <span v-else class="topbar-logo" aria-hidden="true"></span>
          <span class="topbar-title">NVR 控制台</span>
        </router-link>
      </div>

      <div class="topbar-right">
        <Button
            type="button"
            icon="pi pi-sun"
            text
            rounded
            class="topbar-icon-button"
            aria-label="主题"
          />
          <Button
            type="button"
            icon="pi pi-comments"
            text
            rounded
            class="topbar-icon-button"
            aria-label="消息"
          />
          <Button
            type="button"
            icon="pi pi-calendar"
            text
            rounded
            class="topbar-icon-button"
            aria-label="日历"
          />
          <Button
            type="button"
            icon="pi pi-envelope"
            text
            rounded
            class="topbar-icon-button"
            aria-label="邮件"
          />
          <Button
            type="button"
            icon="pi pi-user"
            text
            rounded
            class="topbar-icon-button"
            aria-label="用户菜单"
            @click="toggleUserMenu"
          />
        <Menu ref="userMenu" :model="userMenuItems" :popup="true" />
      </div>
    </header>

    <aside :class="sidebarClass">
      <nav class="layout-menu">
        <div class="layout-menu-section">
          <span class="layout-menu-section-title">主导航</span>
        </div>
        <ul class="layout-menu-list">
          <li v-for="item in menuItems" :key="item.route" class="layout-menuitem">
            <router-link
              :to="item.route"
              class="layout-menuitem-link"
              :class="{ 'active-route': $route.path === item.route }"
            >
              <span :class="['layout-menuitem-icon', item.icon]" />
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
/* 顶栏、侧栏、内容区统一使用同一间距，与参考图一致 */
.layout-wrapper {
  --layout-gap: 1rem;
  --layout-sidebar-width: 14rem;
  --layout-topbar-height: 3rem;
  min-height: 100vh;
  background-color: var(--p-surface-50);
}

.layout-topbar {
  position: fixed;
  left: 0;
  top: 0;
  width: 100%;
  height: var(--layout-topbar-height);
  padding: 0 var(--layout-gap);
  z-index: 997;
  display: flex;
  align-items: center;
  justify-content: space-between;
  background-color: var(--p-content-background);
  border-bottom: 1px solid var(--p-content-border-color);
  transition: left 0.2s;
}

.topbar-left {
  display: flex;
  align-items: center;
  gap: 0.375rem;
}

.topbar-menu-button {
  width: 2rem;
  height: 2rem;
  min-width: 2rem;
}

.topbar-brand {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  text-decoration: none;
  color: var(--p-text-color);
  font-weight: 500;
}

.topbar-logo {
  display: inline-block;
  width: 1.5rem;
  height: 1.5rem;
  min-width: 1.5rem;
  min-height: 1.5rem;
  border-radius: var(--p-content-border-radius, 4px);
  background-color: var(--p-primary-color);
  flex-shrink: 0;
}

.topbar-logo-img {
  display: block;
  width: 1.5rem;
  height: 1.5rem;
  min-width: 1.5rem;
  min-height: 1.5rem;
  object-fit: contain;
  flex-shrink: 0;
}

.topbar-title {
  font-size: 1.125rem;
  margin: 0;
  color: inherit;
}

.topbar-right {
  display: flex;
  align-items: center;
  gap: 0.125rem;
}

.topbar-icon-button {
  width: 2rem;
  height: 2rem;
  min-width: 2rem;
  color: var(--p-text-color);
}

.topbar-icon-button:hover {
  background-color: var(--p-content-hover-background);
  color: var(--p-text-color);
}

/* Sidebar - 与 header 相同间距：左边距 = --layout-gap */
.layout-sidebar {
  position: fixed;
  width: var(--layout-sidebar-width);
  left: var(--layout-gap);
  top: var(--layout-topbar-height);
  height: calc(100vh - var(--layout-topbar-height));
  z-index: 996;
  overflow-y: auto;
  background-color: var(--p-content-background);
  border-radius: var(--p-content-border-radius, 4px);
  padding: 0.5rem var(--layout-gap);
  box-shadow:
    0 1px 3px rgba(0, 0, 0, 0.04),
    0 0 2px rgba(0, 0, 0, 0.06);
  border: 1px solid var(--p-content-border-color);
  transition: transform 0.2s, left 0.2s;
}

.layout-sidebar-collapsed {
  transform: translateX(-100%);
  left: 0;
}

.layout-menu {
  margin: 0;
  padding: 0;
  list-style: none;
}

.layout-menu-section {
  margin: 0.5rem 0;
}

.layout-menu-section-title {
  font-size: 0.75rem;
  font-weight: 700;
  text-transform: uppercase;
  color: var(--p-text-color);
  letter-spacing: 0.05em;
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
  align-items: center;
  padding: 0.5rem 0.75rem;
  color: var(--p-text-color);
  text-decoration: none;
  border-radius: var(--p-content-border-radius, 4px);
  transition: background-color 0.2s;
  cursor: pointer;
  font-size: 0.875rem;
}

.layout-menuitem-link:hover {
  background-color: var(--p-content-hover-background);
}

.layout-menuitem-link.active-route {
  font-weight: 700;
  color: var(--p-primary-color);
  background-color: var(--p-content-hover-background);
}

.layout-menuitem-icon {
  margin-right: 0.5rem;
  font-size: 0.875rem;
  flex-shrink: 0;
}

.layout-menuitem-text {
  font-weight: 500;
}

/* Main content - 与 header/sidebar 相同间距：左右下 = --layout-gap，上 = topbar + gap */
.layout-main-container {
  margin-left: calc(var(--layout-gap) + var(--layout-sidebar-width) + var(--layout-gap));
  padding: calc(var(--layout-topbar-height) + var(--layout-gap)) var(--layout-gap) var(--layout-gap) var(--layout-gap);
  min-height: 100vh;
  transition: margin-left 0.2s;
}

.layout-main-container.layout-sidebar-inactive {
  margin-left: 0;
  padding-left: var(--layout-gap);
}

.layout-main {
  flex: 1;
  padding-bottom: var(--layout-gap);
  overflow: auto;
  min-height: 0;
}

/* 大屏下内容区最大宽度，避免过宽 */
@media (min-width: 1200px) {
  .layout-main-container {
    max-width: 1504px;
  }
}

@media (max-width: 991px) {
  .layout-wrapper {
    --layout-gap: 0.75rem;
  }

  .layout-sidebar {
    left: 0;
    top: 0;
    height: 100vh;
    border-radius: 0;
    border-left: none;
    box-shadow: 2px 0 8px rgba(0, 0, 0, 0.15);
  }

  .layout-sidebar-collapsed {
    transform: translateX(-100%);
  }

  .layout-main-container {
    margin-left: 0;
    padding: calc(var(--layout-topbar-height) + var(--layout-gap)) var(--layout-gap) var(--layout-gap) var(--layout-gap);
  }

  .layout-main-container.layout-sidebar-inactive {
    padding-left: var(--layout-gap);
  }
}
</style>
