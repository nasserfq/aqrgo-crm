// Starts the real app binary against a real Postgres and drives it in Chromium.
// Needs DATABASE_URL (an empty database is fine; migrations run on start).
// APP_BIN defaults to the debug build: `cargo build -p app` first.
const { defineConfig, devices } = require('@playwright/test');
const path = require('path');

const PORT = process.env.E2E_PORT || '8099';
const BIN = process.env.APP_BIN || path.join(__dirname, '..', 'target', 'debug', 'aqrgo-crm');
const LOG = path.join(__dirname, 'server.log');
process.env.E2E_SERVER_LOG = LOG;

module.exports = defineConfig({
  testDir: './tests',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'report' }]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: 'off',
    screenshot: 'only-on-failure',
    locale: 'ar-SA',
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1366, height: 860 } } },
    { name: 'phone', use: { ...devices['Pixel 7'] } },
  ],
  webServer: {
    // Emails go to the log when AWS is not configured; tests read sign-in codes from it.
    command: `"${BIN}" > "${LOG}" 2>&1`,
    url: `http://localhost:${PORT}/healthz`,
    reuseExistingServer: false,
    timeout: 60_000,
    env: {
      PORT,
      PUBLIC_URL: `http://localhost:${PORT}`,
      DATABASE_URL: process.env.DATABASE_URL || '',
      GOOGLE_CLIENT_ID: 'e2e-client',
      GOOGLE_CLIENT_SECRET: 'e2e-secret',
      RUST_LOG: 'info',
    },
  },
});
