import { expect, type Page } from '@playwright/test';

/**
 * Ways into the app that more than one spec needs.
 *
 * ⛔ **The first shared helper module in `e2e/`, and it exists for a reason
 * rather than as tidying.** Opening the Settings dialog and reaching its
 * language pane was written out in three specs — `i18n`, `gallery` and
 * `features` — and every selector in it is a `data-testid` *precisely because
 * the labels are translated*, which `i18n.spec.ts` says at its own top. So a
 * test id renamed in Settings breaks three files that have to be found
 * independently, and the one thing that would make them findable is the thing
 * that was missing.
 *
 * ⚠ Keep this small. A shared module for e2e is a place fixtures accumulate
 * until specs cannot be read on their own; what belongs here is navigation that
 * is genuinely identical everywhere, not assertions.
 */

/**
 * Choose an artist or producer from the roster.
 *
 * ⛔⛔ **The one gesture nearly every spec starts with, and it changed shape on
 * 2026-08-09.** The left rail was a search box plus a five-hundred-row list;
 * it is now a single type-to-filter combobox, and the details pane sits under
 * it. Fifty-odd call sites reached for `getByLabel('Search an artist')` or
 * `.roster__item` directly, so when those went the entire suite failed at its
 * first step — 119 failures, every one of them a 30-second locator timeout for
 * the same reason.
 *
 * ⚠ **Hence a helper rather than fifty updated locators.** The next time the way
 * in changes, this is the one place that has to know.
 *
 * The combobox commits the top match on Enter, which is what makes typing a
 * partial name enough — and is itself the behaviour Mike asked for: *"if i start
 * typing 'Trap' and i get to 'Tra' and click off of it, it should automatically
 * select the first one that was in the list."*
 */
export async function pickArtist(page: Page, name: string): Promise<void> {
  const box = page.getByRole('combobox', { name: 'Roster' });
  await box.click();
  await box.fill(name);
  await page.keyboard.press('Enter');
  // ⛔ **Focus has to leave the box, and this is not tidying.** The app ignores
  // every keyboard shortcut while a field has focus — deliberately, so typing an
  // artist's name cannot fire `K` or `G` (`App.tsx`, and `smoke.spec.ts` pins
  // it). Clicking a roster row used to leave focus on a *button*, so specs could
  // press `1`–`6` or `G` straight afterwards; a combobox leaves the caret in an
  // input, and every one of those specs silently stopped working.
  await box.blur();
}

/** The same, for the genre combobox above it. */
export async function pickGenre(page: Page, name: string): Promise<void> {
  const box = page.getByRole('combobox', { name: 'Genres' });
  await box.click();
  await box.fill(name);
  await page.keyboard.press('Enter');
}

/**
 * The roster combobox itself, for specs that assert on it rather than use it.
 *
 * ⚠ Named here so "what replaced the search box" is answered in one place.
 */
export function rosterBox(page: Page) {
  return page.getByRole('combobox', { name: 'Roster' });
}

/** Open Settings and select the language pane. */
export async function openLanguagePane(page: Page): Promise<void> {
  await page.getByTestId('open-settings').click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.getByTestId('settings-tab-language').click();
}

/**
 * Switch the whole app to `code`, and prove it took.
 *
 * ⚠ **Closed with Escape, not with a button.** Every label in the dialog has
 * just changed language, so a locator that named one would only work in the
 * language it was written in — which is the trap the test ids exist to avoid.
 */
export async function switchLanguage(page: Page, code: string): Promise<void> {
  await openLanguagePane(page);
  await page.getByTestId(`language-${code}`).click();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toBeHidden();
  // The document's own language attribute, which the font stack and the text
  // direction both key off. Wrong here and a screenshot looks plausible while
  // being drawn with the wrong fallback font.
  await expect(page.locator('html')).toHaveAttribute('lang', code);
}
