import { defineConfig, devices } from '@playwright/test'
import { resolve } from 'node:path'

const repositoryRoot = resolve(import.meta.dirname, '../..')

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
const frontendOrigin = `http://127.0.0.1:${frontendPort}`

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: frontendOrigin,
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'mobile-chromium',
      use: { ...devices['Pixel 7'] },
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
          'postgres://hogwarts:local-development-only@127.0.0.1:55432/hogwarts',
        RUST_LOG: 'harry_potter_server=info',
      },
      reuseExistingServer: false,
      timeout: 120_000,
      url: `${backendOrigin}/health/live`,
    },
    {
      command:
        `npm run build && npm exec vite -- preview --host 127.0.0.1 --port ${frontendPort}`,
      cwd: import.meta.dirname,
      env: {
        BACKEND_PROXY_TARGET: backendOrigin,
      },
      reuseExistingServer: false,
      timeout: 30_000,
      url: frontendOrigin,
    },
  ],
})
