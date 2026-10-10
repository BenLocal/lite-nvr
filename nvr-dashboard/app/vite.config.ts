import { fileURLToPath, URL } from 'node:url'

import { defineConfig, loadEnv } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueDevTools from 'vite-plugin-vue-devtools'

// https://vite.dev/config/
// 构建时通过环境变量设置 URL 前缀，例如: VITE_BASE_URL=/nvr/ npm run build
const base = process.env.VITE_BASE_URL ?? '/nvr/'

export default defineConfig(({ mode }) => {
  // Dev-server proxy target: shell env > .env files (see .env.example) > default.
  // No VITE_ prefix, so it stays out of the client bundle.
  const env = loadEnv(mode, fileURLToPath(new URL('.', import.meta.url)), '')
  const apiTarget = env.NVR_API_TARGET || 'http://localhost:18080'

  return {
    base,
    plugins: [
      vue(),
      // 仅开发环境启用，避免生产出现右下角浮动按钮
      process.env.NODE_ENV === 'development' ? vueDevTools() : undefined,
    ].filter(Boolean),
    resolve: {
      alias: {
        '@': fileURLToPath(new URL('./src', import.meta.url))
      },
    },
    server: {
      proxy: {
        '/api': { target: apiTarget, changeOrigin: true },
      },
    },
  }
})
