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
              input-class="field-input-inner"
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

:deep(.p-password.field-input) {
  width: 100%;
}

:deep(.p-password.field-input .p-password-input) {
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
