import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'jsdom',
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    // e2e/ belongs to Playwright, which drives a real browser.
    exclude: ['node_modules', 'dist', 'e2e'],
  },
});
