import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    coverage: {
      provider: 'v8',
      include: [
        'src/forms/detectionForm.ts',
        'src/forms/deviceForm.ts',
        'src/components/DetectionConfigFields.vue',
        'src/components/DetectionOverlay.vue',
      ],
      reporter: ['text', 'html'],
      thresholds: {
        lines: 80,
        functions: 80,
        statements: 80,
      },
    },
  },
})
