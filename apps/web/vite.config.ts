import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5173,
    proxy: {
      '/health': process.env.HEALTH_PROXY_TARGET ?? 'http://127.0.0.1:8080',
    },
  },
})
