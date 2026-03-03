import { computed } from 'vue'
import { clearAuthToken, getAuthToken, setAuthToken } from '../auth/token'

export function useAuth() {
  const isLoggedIn = computed(() => !!getAuthToken())

  function login(token: string, rememberMe = true) {
    setAuthToken(token, rememberMe)
  }

  function getToken() {
    return getAuthToken()
  }

  function logout() {
    clearAuthToken()
  }

  return { isLoggedIn, login, logout, getToken }
}
