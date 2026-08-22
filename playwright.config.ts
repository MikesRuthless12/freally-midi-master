import { defineConfig, devices } from '@playwright/test';

/**
 * The port the dev server and the tests agree on.
 *
 * ⛔⛔ **1420 is not free on every machine, and the failure does not look like
 * one.** On the dev box it belongs to the *Freally File Manager* project's own
 * Vite — and `reuseExistingServer` means Playwright does not fail on the
 * collision, it **reuses it** and runs all three hundred specs against a
 * different application. That produced a wall of unexplainable failures in an
 * earlier `ci:local`, and the cure is a port rather than killing somebody else's
 * server.
 *
 * ⚠ **CI never sets this**, so the default is exactly what the workflow expects
 * and nothing about the gate's own environment changes. Locally:
 * `FMM_E2E_PORT=1431 npm run ci:local`.
 */
const PORT = Number(process.env.FMM_E2E_PORT ?? 1420);

/**
 * E2E against `vite dev` with IPC served by `src/lib/ipc-mock`.
 *
 * Deliberately no plugin binary: the UI is the thing under test here, and
 * building a native bundle per platform to click a tab would make E2E slow
 * enough that people stop running it. What only a real host can show — the
 * editor opening, tempo sync, notes reaching the track — is covered by the
 * `plugin` and `plugin editor` CI jobs and by loading it in a DAW.
 */
export default defineConfig({
  testDir: './e2e',
  /**
   * ⛔ **The gallery is run on demand, not as part of the gate.**
   * `e2e/gallery.spec.ts` asserts almost nothing — it exists to *photograph*
   * every screen and every language so a human can look at them, which is the
   * only way the failures it targets (a label overflowing its chip in German, a
   * right-to-left layout putting the transport in the wrong corner) are ever
   * caught. Nineteen full page loads is real time to add to a gate that already
   * runs on three OSes, and none of it would fail on the things a gate is for.
   *
   * `npm run test:gallery` runs it and leaves the images in
   * `screenshots/gallery/`.
   */
  testIgnore: ['**/gallery.spec.ts', '**/features.spec.ts'],
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
    baseURL: `http://localhost:${PORT}`,
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
    // ⚠ `--strictPort`, so a collision is an error rather than Vite quietly
    // choosing 1421 while `baseURL` still points at 1420.
    command: `npm run dev -- --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    env: { VITE_IPC_MOCK: '1' },
  },
});
