import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

const applicationOrigin = process.env.APPLICATION_ORIGIN ?? 'http://127.0.0.1:5173'
const socketOrigin = applicationOrigin.replace(/^http/, 'ws')
const securityHeaders = {
  'Cache-Control': 'no-store',
  'Referrer-Policy': 'no-referrer',
  'X-Content-Type-Options': 'nosniff',
  'X-Frame-Options': 'DENY',
  'Content-Security-Policy': [
    "default-src 'none'", "script-src 'self'", "style-src 'self'",
    "img-src 'self'", "font-src 'self'", "manifest-src 'self'",
    `connect-src 'self' ${socketOrigin}`, "frame-ancestors 'none'",
    "object-src 'none'", "base-uri 'none'", "form-action 'self'",
  ].join('; '),
}

export default defineConfig(({ command, isPreview }) => ({
  plugins: [vue()],
  // Vite injects styles during HMR. The development nonce is absent from builds/preview.
  html: command === 'serve' && !isPreview ? { cspNonce: 'hogwarts-vite-development' } : {},
  preview: {
    strictPort: true,
    cors: false,
    headers: securityHeaders,
    proxy: {
      '/api': {
        target: process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
        ws: true,
      },
      '/health': process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
    },
  },
  server: {
    strictPort: true,
    cors: false,
    headers: {
      ...securityHeaders,
      'Content-Security-Policy': securityHeaders['Content-Security-Policy']
        .replace("style-src 'self'", "style-src 'self' 'nonce-hogwarts-vite-development'"),
    },
    port: 5173,
    proxy: {
      '/api': {
        target: process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
        ws: true,
      },
      '/health': process.env.BACKEND_PROXY_TARGET ?? 'http://127.0.0.1:8080',
    },
  },
}))
