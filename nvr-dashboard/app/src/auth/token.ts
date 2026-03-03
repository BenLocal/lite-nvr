const AUTH_LOCAL_KEY = 'nvr-auth-token'
const AUTH_SESSION_KEY = 'nvr-auth-token-session'

export function getAuthToken() {
  return localStorage.getItem(AUTH_LOCAL_KEY) || sessionStorage.getItem(AUTH_SESSION_KEY)
}

export function setAuthToken(token: string, rememberMe: boolean) {
  if (rememberMe) {
    localStorage.setItem(AUTH_LOCAL_KEY, token)
    sessionStorage.removeItem(AUTH_SESSION_KEY)
    return
  }
  sessionStorage.setItem(AUTH_SESSION_KEY, token)
  localStorage.removeItem(AUTH_LOCAL_KEY)
}

export function clearAuthToken() {
  localStorage.removeItem(AUTH_LOCAL_KEY)
  sessionStorage.removeItem(AUTH_SESSION_KEY)
}
