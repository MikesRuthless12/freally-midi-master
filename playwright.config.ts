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
  // Measured: the language sweep alone runs 23.0s serially and 12.5s at 4
  // workers, and CI now runs it on three OSes. One worker was a caution that
  // cost ~31s per run across the matrix; these specs share no state.
  workers: process.env.CI ? 4 : undefined,
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
