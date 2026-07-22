import { defineConfig, devices } from '@playwright/test';

/**
 * E2E against `vite dev` with IPC served by `src/lib/ipc-mock`.
 *
 * Deliberately no Tauri binary: the UI is the thing under test here, and
 * building a native binary per platform to click a tab would make E2E slow
 * enough that people stop running it. Native behaviour — drag-out, the crash
 * loop, the updater — is covered by the smoke tests and the Havoc-standard
 * drill instead.
 */
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  // A `.only` left in a spec silently narrows CI to one test.
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : 'list',

  use: {
    baseURL: 'http://localhost:1420',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        // The app's minimum window (PRD § 8), so the right rail is showing.
        viewport: { width: 1600, height: 900 },
      },
    },
  ],

  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:1420',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    env: { VITE_IPC_MOCK: '1' },
  },
});
