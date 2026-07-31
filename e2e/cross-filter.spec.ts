import { expect, test } from '@playwright/test';

/**
 * Genre <-> artist cross-filtering, end to end.
 *
 * `src/lib/cross-filter.test.ts` covers the rules on their own. What only a
 * browser shows is that the rail is actually wired to them — that picking a
 * genre narrows the list the user is looking at, and that a genre nobody works
 * in says so in words instead of rendering an empty panel.
 *
 * The mock roster behind this is two genres and one artist related to one of
 * them (`src/lib/ipc-mock.ts`), so "UK Drill" is a real empty case rather than
 * a contrived one.
 */

const chip = (name: string) => `.chip:text-is("${name}")`;

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('picking a genre narrows the roster to the artists in it', async ({ page }) => {
  await expect(page.locator('.roster__item')).toContainText(['Mock Artist']);
  await expect(page.locator('.roster__filter')).toBeHidden();

  await page.locator(chip('Trap')).click();

  await expect(page.locator('.roster__filter')).toContainText('Filtered by Trap');
  await expect(page.locator('.roster__item', { hasText: 'Mock Artist' })).toBeVisible();
});

test('a genre nobody works in says so rather than showing an empty list', async ({ page }) => {
  await page.locator(chip('UK Drill')).click();

  await expect(page.locator('.rail__hint')).toContainText('No artists work in UK Drill yet.');
  // The artist is gone, but the genres are still there to pick another one —
  // each direction narrows the *other* list, so the rail is never a dead end.
  await expect(page.locator('.roster__item', { hasText: 'Mock Artist' })).toBeHidden();
  await expect(page.locator('.roster__item', { hasText: 'Trap' })).toBeVisible();
});

test('Show all restores the roster without changing the selection', async ({ page }) => {
  await page.locator(chip('UK Drill')).click();
  await expect(page.locator('.rail__hint')).toContainText('No artists work in UK Drill yet.');

  await page.getByRole('button', { name: 'Show all' }).click();

  await expect(page.locator('.roster__item', { hasText: 'Mock Artist' })).toBeVisible();
  await expect(page.locator('.roster__filter')).toBeHidden();
  // Still selected — this clears the filter, not the choice the filter came from.
  await expect(page.locator(chip('UK Drill'))).toHaveAttribute('aria-pressed', 'true');
});

test('picking an artist shows only the genres it works in', async ({ page }) => {
  await page.locator('.roster__item', { hasText: 'Mock Artist' }).click();

  await expect(page.locator('.roster__filter')).toContainText('Filtered by Mock Artist');
  await expect(page.locator(chip('Trap'))).toBeVisible();
  await expect(page.locator(chip('UK Drill'))).toBeHidden();
});
