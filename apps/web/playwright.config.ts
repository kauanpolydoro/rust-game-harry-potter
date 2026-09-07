import { defineConfig, devices } from '@playwright/test'
import { resolve } from 'node:path'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'

const repositoryRoot = resolve(import.meta.dirname, '../..')

// Safari requires HTTPS to send the production Secure session cookie.
// Each runner owns a short-lived certificate; workers inherit its directory.
if (!process.env.E2E_TLS_DIRECTORY) {
  const directory = mkdtempSync(resolve(tmpdir(), 'hogwarts-e2e-tls-'))
  process.env.E2E_TLS_DIRECTORY = directory
  process.once('exit', () => rmSync(directory, { recursive: true, force: true }))
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes',
    '-keyout', resolve(directory, 'key.pem'), '-out', resolve(directory, 'cert.pem'),
    '-days', '2', '-subj', '/CN=localhost', '-addext', 'subjectAltName=IP:127.0.0.1,DNS:localhost'],
  { stdio: 'ignore' })
}

function localPort(environmentName: string, fallback: number): number {
  const configuredValue = process.env[environmentName]
  if (configuredValue === undefined) {
    return fallback
  }
  const port = Number(configuredValue)
  if (!Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new TypeError(`${environmentName} must be an integer between 1 and 65535`)
  }
  return port
}

const backendPort = localPort('E2E_BACKEND_PORT', 18_080)
const frontendPort = localPort('E2E_FRONTEND_PORT', 4_173)
const backendOrigin = `http://127.0.0.1:${backendPort}`
const frontendOrigin = `https://127.0.0.1:${frontendPort}`

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  // Concurrent software-rendered scenes otherwise starve each other's input loop.
  workers: 2,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: frontendOrigin,
    ignoreHTTPSErrors: true,
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'mobile-chromium',
      use: { ...devices['Pixel 7'] },
    },
    {
      name: 'table-webkit',
      testMatch: 'table3d.spec.ts',
      use: { ...devices['iPhone 13'], viewport: { width: 844, height: 390 } },
    },
    {
      name: 'table-firefox',
      testMatch: 'table3d.spec.ts',
      // Linux software WebGL needs a display (use xvfb-run on a headless host).
      use: { browserName: 'firefox', headless: false, viewport: { width: 1366, height: 768 } },
    },
  ],
  webServer: [
    {
      command:
        'cargo build --release -p harry-potter-server --example e2e_harness && ./target/release/examples/e2e_harness',
      cwd: repositoryRoot,
      env: {
        APPLICATION_ORIGIN: frontendOrigin,
        BIND_ADDRESS: `127.0.0.1:${backendPort}`,
        DATABASE_URL:
          process.env.TEST_DATABASE_URL ??
          `postgres://hogwarts:local-development-only@127.0.0.1:${localPort('POSTGRES_PORT', 55_432)}/hogwarts`,
        RUST_LOG: 'harry_potter_server=info',
      },
      reuseExistingServer: false,
      // A cold Rust build after a toolchain update precedes backend readiness.
      timeout: 600_000,
      url: `${backendOrigin}/health/live`,
    },
    {
      command:
        `npm run build && npm exec vite -- preview --host 127.0.0.1 --port ${frontendPort}`,
      cwd: import.meta.dirname,
      env: {
        BACKEND_PROXY_TARGET: backendOrigin,
        APPLICATION_ORIGIN: frontendOrigin,
        E2E_TLS_DIRECTORY: process.env.E2E_TLS_DIRECTORY,
      },
      reuseExistingServer: false,
      timeout: 30_000,
      url: frontendOrigin,
      ignoreHTTPSErrors: true,
    },
  ],
})
