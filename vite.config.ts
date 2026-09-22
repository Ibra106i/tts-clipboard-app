/// <reference types="vitest/config" />
import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

const mock = (relativePath: string) =>
  fileURLToPath(new URL(relativePath, import.meta.url))

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    restoreMocks: true,
    clearMocks: true,
    // The real Tauri APIs can only run inside a webview, so the test suite
    // substitutes in-process fakes. Tests drive them through
    // `src/test/mocks/harness.ts`.
    alias: {
      '@tauri-apps/api/core': mock('./src/test/mocks/tauri-core.ts'),
      '@tauri-apps/api/event': mock('./src/test/mocks/tauri-event.ts'),
      '@tauri-apps/api/window': mock('./src/test/mocks/tauri-window.ts'),
      '@tauri-apps/api/webview': mock('./src/test/mocks/tauri-webview.ts'),
    },
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,tsx}'],
      exclude: [
        'src/main.tsx',
        'src/test/**',
        'src/**/*.test.{ts,tsx}',
        'src/vite-env.d.ts',
      ],
    },
  },
})
