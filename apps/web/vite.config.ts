import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'
import { existsSync, readFileSync } from 'node:fs'
import onHeaders from 'on-headers'
import { resolve } from 'node:path'

const applicationOrigin = process.env.APPLICATION_ORIGIN ?? 'http://127.0.0.1:5173'
const socketOrigin = applicationOrigin.replace(/^http/, 'ws')
const securityHeaders = {
  'Referrer-Policy': 'no-referrer',
  'X-Content-Type-Options': 'nosniff',
  'X-Frame-Options': 'DENY',
  'Content-Security-Policy': [
    "default-src 'none'", "script-src 'self' 'wasm-unsafe-eval'", "worker-src 'self'", "style-src 'self'",
    "img-src 'self'", "font-src 'self'", "manifest-src 'self'",
    `connect-src 'self' ${socketOrigin}`, "frame-ancestors 'none'",
    "object-src 'none'", "base-uri 'none'", "form-action 'self'",
  ].join('; '),
}

export default defineConfig(({ command, isPreview }) => ({
  plugins: [vue(), {
    name: 'table-public-asset-cache',
    configurePreviewServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = new URL(request.url ?? '/', applicationOrigin).pathname
        const immutable = /^\/assets\/[^/]+$/.test(path) || /^\/table-(assets|audio)\/v[0-9]+\//.test(path)
        const revalidate = /^\/(table-art|textures)\//.test(path)
        const fileExists = existsSync(resolve(server.config.root, server.config.build.outDir, `.${path}`))
        // Run at final header emission: Vite's HTML sender otherwise overwrites
        // middleware headers, including for routes served by the SPA fallback.
        onHeaders(response, function () {
          const html = String(this.getHeader('Content-Type') ?? '').includes('text/html')
          const successful = this.statusCode >= 200 && this.statusCode < 400
          this.setHeader('Cache-Control', !fileExists || html || !successful ? 'no-store'
            : immutable ? 'public, max-age=31536000, immutable'
              : revalidate ? 'public, max-age=0, must-revalidate' : 'no-store')
        })
        next()
      })
    },
  }],
  worker: { format: 'es' },
  build: { manifest: true, rollupOptions: { input: { app: 'index.html', prototype: 'prototype.html' } } },
  // Vite injects styles during HMR. The development nonce is absent from builds/preview.
  html: command === 'serve' && !isPreview ? { cspNonce: 'hogwarts-vite-development' } : {},
  preview: {
    https: process.env.E2E_TLS_DIRECTORY ? {
      key: readFileSync(resolve(process.env.E2E_TLS_DIRECTORY, 'key.pem')),
      cert: readFileSync(resolve(process.env.E2E_TLS_DIRECTORY, 'cert.pem')),
    } : undefined,
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
      'Cache-Control': 'no-store',
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
