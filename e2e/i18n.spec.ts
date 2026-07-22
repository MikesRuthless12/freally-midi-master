import { expect, test, type Page } from '@playwright/test';

import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { LOCALES } from '../src/i18n/locales';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

// Read rather than `import ... from './en.json'`: Playwright runs this file as
// a native ES module, where a JSON import needs an import attribute Vite's
// bundler does not emit.
function catalog(code: string) {
  return JSON.parse(
    readFileSync(join(root, 'src', 'i18n', 'locales', `${code}.json`), 'utf8'),
  ) as {
    stage: { emptyTitle: string };
    settings: {
      title: string;
      general: string;
      appearance: string;
      language: string;
      about: string;
      languageHeading: string;
    };
  };
}

const en = catalog('en');

/** Where the per-language screenshots land for the CI artifact. */
const SHOTS = join(root, 'screenshots', 'i18n');
mkdirSync(SHOTS, { recursive: true });

/**
 * Every language switch, in a real browser.
 *
 * `src/i18n/locales.test.ts` proves the catalogs are complete and
 * `src/assets/fonts/fonts.test.ts` proves the glyphs exist. Neither proves the
 * app actually *renders* in that language — that the picker works, that the
 * strings reach the DOM, that a font which can draw the script is the one
 * chosen, and that Arabic flips the layout. This does, once per locale.
 *
 * The font assertion is why this belongs in a browser at all: only a real
 * engine will say which family it picked out of a stack of 27. A jsdom test
 * would report the whole stack back and prove nothing.
 *
 * Selectors are `data-testid`, deliberately. Every label in this UI is
 * translated, so a test that finds the Settings button by its accessible name
 * can only run in the one language that name was written in.
 */

async function openLanguagePane(page: Page) {
  await page.getByTestId('open-settings').click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.getByTestId('settings-tab-language').click();
}

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test.describe('language switching', () => {
  test('offers every canonical locale by its native name', async ({ page }) => {
    await openLanguagePane(page);
    for (const { code, native } of LOCALES) {
      // Literal, not a pattern: "Português (Brasil)" has regex parentheses in
      // it, and as a pattern it silently matched "Português Brasil" instead.
      await expect(page.getByTestId(`language-${code}`)).toContainText(native);
    }
  });

  test('lists English first so the order never shifts under the user', async ({ page }) => {
    // Sorting by endonym would reorder the list on every switch, so nobody
    // could learn where their own language sits.
    await openLanguagePane(page);
    await expect(page.locator('.settings__language').first()).toContainText('English');
  });

  /**
   * The Settings modal must translate ITSELF, not just the app behind it.
   *
   * This is the screen the switch happens on, so it is the one place a user can
   * immediately tell whether the choice took. If the modal keeps its English
   * chrome while the app behind it changes, the picker looks broken even though
   * it worked.
   *
   * Each run leaves a screenshot in screenshots/i18n/ for the CI artifact, so
   * the whole set can be eyeballed on the real runners rather than inferred
   * from a green tick.
   */
  for (const { code, native, english } of LOCALES.filter((l) => l.code !== 'en')) {
    test(`Settings switches to ${english} (${code})`, async ({ page }) => {
      await openLanguagePane(page);

      // Baseline: the modal is in English before the switch.
      await expect(page.getByRole('heading', { name: en.settings.title })).toBeVisible();

      await page.getByTestId(`language-${code}`).click();

      const t = catalog(code);
      // The modal's own title and every category label, in the new language.
      await expect(page.getByRole('heading', { name: t.settings.title })).toBeVisible();
      for (const [id, label] of [
        ['general', t.settings.general],
        ['appearance', t.settings.appearance],
        ['language', t.settings.language],
        ['about', t.settings.about],
      ] as const) {
        await expect(page.getByTestId(`settings-tab-${id}`)).toContainText(label);
      }
      // ...and the open pane's own heading.
      await expect(
        page.getByRole('heading', { name: t.settings.languageHeading }),
      ).toBeVisible();

      // The document declares the language: it drives the browser's own font
      // selection and hyphenation, not just screen readers.
      await expect(page.locator('html')).toHaveAttribute('lang', code);
      await expect(page.locator('html')).toHaveAttribute('dir', code === 'ar' ? 'rtl' : 'ltr');

      // Drawn with a font that can draw it. `font-family` resolves to the whole
      // stack, so ask the engine what it actually settled on.
      const panel = page.locator('.settings__panel');
      const family = await panel.evaluate((el) => getComputedStyle(el).fontFamily);
      expect(family, `${english} must render in a bundled Noto face`).toContain('Noto Sans');

      // Nothing may render as tofu. The replacement character is what a browser
      // substitutes when a codepoint decoded badly, and it is the one failure
      // invisible to a reviewer who cannot read the language.
      expect(await panel.innerText()).not.toContain('�');

      // Evidence, per language, for a human to look at.
      await panel.screenshot({ path: join(SHOTS, `settings-${code}.png`) });

      // Behind the modal, the app switched too.
      await page.getByTestId('settings-close').click();
      const headline = page.locator('.stage__empty h2');
      await expect(headline).toBeVisible();
      const text = (await headline.innerText()).trim();
      expect(text.length).toBeGreaterThan(0);
      expect(text, `${english} still shows the English headline`).not.toBe(en.stage.emptyTitle);
      expect(native.length).toBeGreaterThan(0);
    });
  }

  test('English stays English', async ({ page }) => {
    // The control case. Without it, a picker that broke every locale into the
    // same wrong language would still pass every test above.
    await openLanguagePane(page);
    await page.getByTestId('language-en').click();
    await expect(page.getByRole('heading', { name: en.settings.title })).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');

    await page.getByTestId('settings-close').click();
    await expect(page.locator('.stage__empty h2')).toHaveText(en.stage.emptyTitle);
  });

  test('survives a reload', async ({ page }) => {
    // The choice is persisted, so a restart must not silently revert it — the
    // most common way a language picker "works" and then does not.
    await openLanguagePane(page);
    await page.getByTestId('language-ja').click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');

    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
  });

  test('flips the layout for Arabic and back', async ({ page }) => {
    await openLanguagePane(page);
    await page.getByTestId('language-ar').click();
    await expect(page.locator('html')).toHaveAttribute('dir', 'rtl');

    await page.getByTestId('language-en').click();
    await expect(page.locator('html')).toHaveAttribute('dir', 'ltr');
  });
});
