import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test, type Page } from '@playwright/test';

import { switchLanguage } from './app';
import { LOCALES } from '../src/i18n/locales';

/**
 * A gallery of the whole app, to be looked at (Mike, 2026-08-06).
 *
 * *"is there any way to test every single feature of this app with chromium
 * except for dragging/dropping midi (including screenshots) so that way i can
 * see if the language switches when i switch languages and that everything
 * works properly?"*
 *
 * ⚠ **This asserts almost nothing on purpose, and that is the difference
 * between it and every other spec here.** The rest of the suite proves
 * behaviour; `i18n.spec.ts` already proves each language reaches the DOM and
 * picks a font that can draw it. What none of them can do is let a human *look*
 * — and the failures this catches are the ones only a human sees: a label that
 * overflows its chip in German, a Japanese line that wraps a button onto two
 * rows, a right-to-left layout that puts the transport in the wrong corner.
 *
 * ⛔ **`i18n.spec.ts` shoots the Settings panel only**, which is where the
 * language picker lives — so it proves the picker works and shows nothing about
 * the app behind it. These are the screens a producer actually spends their
 * time on.
 *
 * ⚠ **The drag-out is absent and cannot be here.** It hands files to the OS, so
 * it leaves the browser entirely; there is nothing for Chromium to photograph.
 * That one stays Mike's, in a real DAW.
 */

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const SHOTS = join(root, 'screenshots', 'gallery');

/** Pick an artist and fill every generator, so the screens have real content. */
async function loadASession(page: Page) {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  const search = page.getByRole('combobox', { name: 'Roster' });
  await search.fill('trap');
  await search.press('Enter');

  // ⚠ Generate *all*, so the melodic tabs and the stem rows have something in
  // them. A gallery of empty-state screens would photograph the one thing that
  // is already covered by the "ready, hit Generate" tests.
  await page.getByRole('button', { name: 'Generate all' }).click();
  // ⚠ **Waited for on a melodic tab, not on Drums.** The app opens on Drums,
  // which draws `DrumGrid` — the roll's note list does not exist there at all,
  // so waiting for it from the default tab never resolves.
  await page.getByRole('tab', { name: 'Melody' }).click();
  await expect(page.locator('[data-testid="roll-notes"] li').first()).toBeAttached({
    timeout: 15_000,
  });
}

test.describe('the gallery', () => {
  /**
   * One image per screen, in English, at the app's own minimum width.
   *
   * ⚠ Full-page rather than element shots: what goes wrong at this level is
   * *layout*, and cropping to one panel is exactly what would hide it.
   */
  test('photographs every screen a producer uses', async ({ page }) => {
    await loadASession(page);

    for (const tab of ['Drums', 'Melody', 'Counter', 'Bass', 'Chords'] as const) {
      await page.getByRole('tab', { name: tab }).click();
      await page.screenshot({ path: join(SHOTS, `view-${tab.toLowerCase()}.png`) });
    }

    // Song Mode arranges rather than generating a part, so it needs its own
    // press before there is anything to photograph.
    await page.getByRole('tab', { name: 'Song' }).click();
    await page.getByRole('button', { name: 'Generate', exact: true }).click();
    await expect(page.locator('[data-testid="song-ruler"]')).toBeVisible({ timeout: 15_000 });
    await page.screenshot({ path: join(SHOTS, 'view-song.png') });

    // The two panels that hold the ways out of the app.
    await page.getByTestId('open-settings').click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.screenshot({ path: join(SHOTS, 'view-settings.png') });
  });

  /**
   * One image of the working app per language — not of the picker.
   *
   * ⛔ **English is included**, unlike `i18n.spec.ts` which skips it as the
   * baseline. Here it is the thing every other image is compared against, so
   * leaving it out would mean opening two folders to judge one language.
   */
  for (const { code, english } of LOCALES) {
    test(`photographs the app in ${english} (${code})`, async ({ page }) => {
      await loadASession(page);

      // ⚠ English is the baseline and needs no switch. `switchLanguage` also
      // asserts the document's own `lang`, which is what the font stack and the
      // text direction both key off — wrong there and a screenshot looks
      // plausible while being drawn with the wrong fallback font.
      if (code !== 'en') await switchLanguage(page, code);
      await expect(page.locator('html')).toHaveAttribute('lang', code);
      await page.screenshot({ path: join(SHOTS, `app-${code}.png`) });
    });
  }
});
