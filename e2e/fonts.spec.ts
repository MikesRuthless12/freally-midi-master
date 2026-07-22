import { expect, test } from '@playwright/test';

/**
 * The bundled fonts must actually LOAD.
 *
 * `src/assets/fonts/fonts.test.ts` checks the catalogs' unicode-ranges and that
 * every referenced file exists on disk. Neither says whether the browser
 * accepted the `@font-face` rule — and it did not: the vendoring script emitted
 * `src: url(...) format('woff2') format('woff2')`, a duplicated `format()`,
 * which is invalid `<font-src>` grammar. Chromium drops the whole `src`
 * descriptor, so all 546 faces were inert and every language fell through to
 * whatever the OS had.
 *
 * The e2e sweep could not catch it either: it asserted on
 * `getComputedStyle().fontFamily`, which returns the declared *stack* string
 * and contains "Noto Sans" whether or not a single byte of it ever loaded.
 *
 * So this asks the font engine directly, which is the only thing that knows.
 */

test('the bundled Noto faces are accepted and loaded by the browser', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  const report = await page.evaluate(async () => {
    await document.fonts.ready;

    // Every @font-face the page actually parsed. A rule whose `src` was
    // rejected never becomes a FontFace at all.
    const faces = [...document.fonts];
    const families = new Set(faces.map((f) => f.family.replace(/['"]/g, '')));

    // Force the Latin face to load and report whether the engine can use it.
    await document.fonts.load('400 16px "Noto Sans"', 'Generate MIDI');
    return {
      faceCount: faces.length,
      families: [...families].sort(),
      canUseNotoSans: document.fonts.check('400 16px "Noto Sans"', 'Generate MIDI'),
    };
  });

  expect(report.faceCount, 'no @font-face rule survived CSS parsing').toBeGreaterThan(0);
  expect(report.families).toContain('Noto Sans');
  expect(report.canUseNotoSans, 'Noto Sans is declared but the engine cannot use it').toBe(
    true,
  );
});
