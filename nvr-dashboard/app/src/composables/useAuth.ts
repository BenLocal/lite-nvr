import { computed } from 'vue'

const AUTH_KEY = 'nvr-auth-token'

export function useAuth() {
  const isLoggedIn = computed(() => !!localStorage.getItem(AUTH_KEY))

  function login(token: string) {
    localStorage.setItem(AUTH_KEY, token)
  }

  function logout() {
    localStorage.removeItem(AUTH_KEY)
  }

  return { isLoggedIn, login, logout }
}
