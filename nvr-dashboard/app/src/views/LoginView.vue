<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import Form from '@primevue/forms/form'
import InputText from 'primevue/inputtext'
import Password from 'primevue/password'
import Button from 'primevue/button'
import Checkbox from 'primevue/checkbox'
import Message from 'primevue/message'
import { useAuth } from '../composables/useAuth'
import { loginByPassword } from '../api/user'

const router = useRouter()
const { login } = useAuth()

const error = ref('')
const loading = ref(false)

const initialValues = {
  username: '',
  password: '',
  rememberMe: false,
}

function resolver({ values }: { values: Record<string, unknown> }) {
  const errors: Record<string, { message: string }[]> = {}
  const username = String(values.username ?? '').trim()
  const password = String(values.password ?? '')

  if (!username) {
    errors.username = [{ message: '请输入用户名' }]
  }
  if (!password) {
    errors.password = [{ message: '请输入密码' }]
  }

  return {
    values: {
      ...values,
      username,
      password,
    },
    errors,
  }
}

async function onSubmit(event: { valid: boolean; values: Record<string, unknown> }) {
  error.value = ''
  if (!event.valid) {
    return
  }
  loading.value = true

  try {
    const data = await loginByPassword({
      username: String(event.values.username ?? ''),
      password: String(event.values.password ?? ''),
    })

    login(data.token, Boolean(event.values.rememberMe))
    loading.value = false
    const redirect = (router.currentRoute.value.query.redirect as string) || '/'
    router.push(redirect)
  } catch {
    loading.value = false
    error.value = '用户名或密码错误'
  }
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

        <Form v-slot="$form" :resolver="resolver" :initial-values="initialValues" class="login-form" @submit="onSubmit">
          <Message v-if="error" severity="error" :closable="false" class="login-error">
            {{ error }}
          </Message>

          <div class="field">
            <label for="username">用户名</label>
            <InputText
              id="username"
              name="username"
              type="text"
              placeholder="请输入用户名"
              class="field-input"
              :invalid="$form.username?.invalid"
              autocomplete="username"
            />
            <Message
              v-if="$form.username?.invalid"
              severity="error"
              size="small"
              variant="simple"
            >
              {{ $form.username.error?.message }}
            </Message>
          </div>

          <div class="field">
            <label for="password">密码</label>
            <Password
              id="password"
              name="password"
              placeholder="请输入密码"
              :feedback="false"
              toggle-mask
              class="field-input"
              :invalid="$form.password?.invalid"
              autocomplete="current-password"
            />
            <Message
              v-if="$form.password?.invalid"
              severity="error"
              size="small"
              variant="simple"
            >
              {{ $form.password.error?.message }}
            </Message>
          </div>

          <div class="login-options">
            <div class="remember-me">
              <Checkbox name="rememberMe" input-id="remember" :binary="true" />
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
        </Form>
      </div>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  display: flex;
  min-height: 100vh;
  background: linear-gradient(135deg, #0f172a 0%, #1e293b 100%);
}

.login-left {
  flex: 1;
  background: linear-gradient(135deg, rgb(59 130 246 / 10%) 0%, rgb(37 99 235 / 5%) 100%);
  backdrop-filter: blur(20px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 3rem;
  border-right: 1px solid rgb(148 163 184 / 10%);
  position: relative;
  overflow: hidden;
}

.login-left::before {
  content: '';
  position: absolute;
  top: -50%;
  left: -50%;
  width: 200%;
  height: 200%;
  background: radial-gradient(circle, rgb(59 130 246 / 10%) 0%, transparent 70%);
  animation: rotate 20s linear infinite;
}

@keyframes rotate {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}

.login-left-content {
  max-width: 28rem;
  color: #e2e8f0;
  position: relative;
  z-index: 1;
}

.login-brand {
  margin-bottom: 2rem;
  display: flex;
  align-items: center;
  gap: 1rem;
}

.login-brand::before {
  content: '';
  display: block;
  width: 3rem;
  height: 3rem;
  background: linear-gradient(135deg, #3b82f6 0%, #2563eb 100%);
  border-radius: 0.75rem;
  box-shadow: 0 8px 24px rgb(59 130 246 / 40%);
}

.brand-text {
  font-size: 2.5rem;
  font-weight: 700;
  letter-spacing: -0.02em;
  background: linear-gradient(135deg, #e2e8f0 0%, #94a3b8 100%);
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

.login-tagline {
  font-size: 1rem;
  line-height: 1.6;
  color: #94a3b8;
  margin: 0;
}

.login-right {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 3rem;
}

.login-form-wrapper {
  width: 100%;
  max-width: 22rem;
  padding: 2.5rem;
  background: rgb(15 23 42 / 60%);
  backdrop-filter: blur(12px);
  border: 1px solid rgb(148 163 184 / 10%);
  border-radius: 1rem;
  box-shadow: 0 8px 32px rgb(0 0 0 / 30%);
}

.login-title {
  margin: 0 0 0.5rem;
  font-size: 1.5rem;
  font-weight: 600;
  color: #e2e8f0;
  letter-spacing: -0.025em;
}

.login-subtitle {
  margin: 0 0 2rem;
  font-size: 0.875rem;
  color: #94a3b8;
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}

.login-error {
  margin-bottom: 0.25rem;
  background: rgb(239 68 68 / 10%);
  border-color: rgb(239 68 68 / 30%);
  color: #fca5a5;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.field label {
  font-weight: 500;
  font-size: 0.8125rem;
  color: #cbd5e1;
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
  font-size: 0.8125rem;
  color: #cbd5e1;
  cursor: pointer;
}

.forgot-link {
  font-size: 0.8125rem;
  color: #60a5fa;
  text-decoration: none;
  transition: color 0.2s;
}

.forgot-link:hover {
  color: #3b82f6;
  text-decoration: underline;
}

.login-button {
  width: 100%;
  margin-top: 0.5rem;
  background: linear-gradient(135deg, #3b82f6 0%, #2563eb 100%);
  border: none;
  box-shadow: 0 4px 12px rgb(59 130 246 / 30%);
  transition: all 0.3s;
}

.login-button:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 20px rgb(59 130 246 / 40%);
}

@media (width <= 768px) {
  .login-page {
    flex-direction: column;
  }

  .login-left {
    min-height: 16rem;
    padding: 2rem 1.5rem;
    border-right: none;
    border-bottom: 1px solid rgb(148 163 184 / 10%);
  }

  .brand-text {
    font-size: 2rem;
  }

  .login-tagline {
    font-size: 0.875rem;
  }

  .login-right {
    padding: 2rem 1.5rem;
  }

  .login-form-wrapper {
    padding: 2rem;
  }
}
</style>
