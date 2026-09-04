import { defineConfig, devices } from '@playwright/test'
import { resolve } from 'node:path'

const repositoryRoot = resolve(import.meta.dirname, '../..')

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: 'http://127.0.0.1:4173',
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
        APPLICATION_ORIGIN: 'http://127.0.0.1:4173',
        BIND_ADDRESS: '127.0.0.1:18080',
        DATABASE_URL:
          process.env.TEST_DATABASE_URL ??
          'postgres://hogwarts:local-development-only@127.0.0.1:55432/hogwarts',
        RUST_LOG: 'harry_potter_server=info',
      },
      reuseExistingServer: false,
      timeout: 120_000,
      url: 'http://127.0.0.1:18080/health/live',
    },
    {
      command:
        'npm run build && npm exec vite -- preview --host 127.0.0.1 --port 4173',
      cwd: import.meta.dirname,
      env: {
        BACKEND_PROXY_TARGET: 'http://127.0.0.1:18080',
      },
      reuseExistingServer: false,
      timeout: 30_000,
      url: 'http://127.0.0.1:4173',
    },
  ],
})
