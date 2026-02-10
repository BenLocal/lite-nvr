import { createRouter, createWebHistory } from 'vue-router'
import { useAuth } from '../composables/useAuth'

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    {
      path: '/login',
      name: 'login',
      component: () => import('../views/LoginView.vue'),
      meta: { guest: true },
    },
    {
      path: '/',
      component: () => import('../layouts/AppLayout.vue'),
      meta: { requiresAuth: true },
      children: [
        {
          path: '',
          name: 'home',
          component: () => import('../views/DashboardView.vue'),
        },
        {
          path: 'devices',
          name: 'devices',
          component: () => import('../views/DeviceListView.vue'),
        },
      ],
    },
  ],
})

router.beforeEach((to) => {
  const { isLoggedIn } = useAuth()
  const requiresAuth = to.matched.some((r) => r.meta.requiresAuth)
  const guestOnly = to.matched.some((r) => r.meta.guest)

  if (requiresAuth && !isLoggedIn.value) {
    return { name: 'login', query: { redirect: to.fullPath } }
  }
  if (guestOnly && isLoggedIn.value) {
    return { name: 'home' }
  }
  return true
})

export default router
