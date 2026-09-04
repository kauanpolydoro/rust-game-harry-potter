import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [vue()],
  preview: {
    proxy: {
      '/api': {
        target: process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
        ws: true,
      },
      '/health': process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
        ws: true,
      },
      '/health': process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
    },
  },
})
