import { defineConfig } from '@playwright/test';

import base from './playwright.config';

/**
 * The same suite on a port this machine actually has free.
 *
 * ⛔⛔ **Port 1420 belongs to the *Freally File Manager* project's own Vite on
 * the dev machine**, and `reuseExistingServer` means Playwright does not fail on
 * the collision — it *reuses* it and runs the whole suite against a different
 * application. That is what produced the e2e and gallery "failures" in an
 * earlier `ci:local`, and it is not a test failure.
 *
 * ⚠ **Not the CI config.** CI runners have 1420 free and `reuseExistingServer`
 * is off there, so `playwright.config.ts` stays exactly as the gate reads it.
 * Run this one by hand: `npx playwright test --config playwright.local.config.ts`.
 */
const PORT = Number(process.env.FMM_E2E_PORT ?? 1431);

export default defineConfig({
  ...base,
  use: { ...base.use, baseURL: `http://localhost:${PORT}` },
  webServer: {
    ...base.webServer,
    command: `npm run dev -- --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
  },
});
