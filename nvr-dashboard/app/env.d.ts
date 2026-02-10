/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** 构建时 URL 前缀，默认 /nvr/，在 vite.config 或 VITE_BASE_URL 环境变量中设置 */
  readonly VITE_BASE_URL: string
}
