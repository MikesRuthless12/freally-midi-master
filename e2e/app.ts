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
