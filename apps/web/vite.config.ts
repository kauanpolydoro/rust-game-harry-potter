import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5173,
    proxy: {
      '/api': process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
      '/health': process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
    },
  },
})
