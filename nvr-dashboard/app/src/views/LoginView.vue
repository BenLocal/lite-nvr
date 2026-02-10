<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import InputText from 'primevue/inputtext'
import Password from 'primevue/password'
import Button from 'primevue/button'
import Checkbox from 'primevue/checkbox'
import Message from 'primevue/message'
import { useAuth } from '../composables/useAuth'

const router = useRouter()
const { login } = useAuth()

const username = ref('')
const password = ref('')
const rememberMe = ref(false)
const error = ref('')
const loading = ref(false)

function onSubmit() {
  error.value = ''
  if (!username.value.trim()) {
    error.value = '请输入用户名'
    return
  }
  if (!password.value) {
    error.value = '请输入密码'
    return
  }
  loading.value = true
  setTimeout(() => {
    login('token-' + Date.now())
    loading.value = false
    const redirect = (router.currentRoute.value.query.redirect as string) || '/'
    router.push(redirect)
  }, 400)
}
</script>

<template>
  <div class="login-page">
    <div class="login-left">
      <div class="login-left-content">
        <div class="login-brand">
          <span class="brand-text">NVR</span>
        </div>
        <p class="login-tagline">网络视频录像控制台，安全可靠。</p>
      </div>
    </div>

    <div class="login-right">
      <div class="login-form-wrapper">
        <h1 class="login-title">欢迎使用 NVR 控制台</h1>
        <p class="login-subtitle">登录以继续</p>

        <form class="login-form" @submit.prevent="onSubmit">
          <Message v-if="error" severity="error" :closable="false" class="login-error">
            {{ error }}
          </Message>

          <div class="field">
            <label for="username">用户名</label>
            <InputText
              id="username"
              v-model="username"
              type="text"
              placeholder="请输入用户名"
              class="field-input"
              autocomplete="username"
            />
          </div>

          <div class="field">
            <label for="password">密码</label>
            <Password
              id="password"
              v-model="password"
              placeholder="请输入密码"
              :feedback="false"
              toggle-mask
              class="field-input"
              input-class="field-input-inner"
              autocomplete="current-password"
            />
          </div>

          <div class="login-options">
            <div class="remember-me">
              <Checkbox v-model="rememberMe" input-id="remember" :binary="true" />
              <label for="remember">记住我</label>
            </div>
            <a href="#" class="forgot-link" @click.prevent>忘记密码？</a>
          </div>

          <Button
            type="submit"
            label="登录"
            :loading="loading"
            class="login-button"
          />
        </form>
      </div>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  display: flex;
  min-height: 100vh;
}

.login-left {
  flex: 1;
  background: linear-gradient(135deg, var(--p-primary-color) 0%, color-mix(in srgb, var(--p-primary-color) 80%, black) 100%);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem;
}

.login-left-content {
  max-width: 28rem;
  color: rgba(255, 255, 255, 0.95);
}

.login-brand {
  margin-bottom: 1.5rem;
}

.brand-text {
  font-size: 3rem;
  font-weight: 700;
  letter-spacing: -0.02em;
}

.login-tagline {
  font-size: 1.125rem;
  line-height: 1.6;
  opacity: 0.9;
}

.login-right {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem;
  background: var(--p-surface-0);
}

.login-form-wrapper {
  width: 100%;
  max-width: 22rem;
}

.login-title {
  margin: 0 0 0.5rem;
  font-size: 1.75rem;
  font-weight: 700;
  color: var(--p-text-color);
}

.login-subtitle {
  margin: 0 0 2rem;
  font-size: 1rem;
  color: var(--p-text-muted-color);
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}

.login-error {
  margin-bottom: 0.25rem;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.field label {
  font-weight: 500;
  font-size: 0.875rem;
  color: var(--p-text-color);
}

.field-input {
  width: 100%;
}

.field-input-inner {
  width: 100%;
}

.login-options {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.remember-me {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.remember-me label {
  font-size: 0.875rem;
  color: var(--p-text-color);
  cursor: pointer;
}

.forgot-link {
  font-size: 0.875rem;
  color: var(--p-primary-color);
  text-decoration: none;
}

.forgot-link:hover {
  text-decoration: underline;
}

.login-button {
  width: 100%;
  margin-top: 0.25rem;
}

@media (max-width: 768px) {
  .login-page {
    flex-direction: column;
  }

  .login-left {
    min-height: 12rem;
    padding: 2rem 1.5rem;
  }

  .brand-text {
    font-size: 2rem;
  }

  .login-tagline {
    font-size: 1rem;
  }

  .login-right {
    padding: 1.5rem;
  }
}
</style>
