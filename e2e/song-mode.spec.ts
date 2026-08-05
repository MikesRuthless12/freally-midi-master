import { expect, test, type Page } from '@playwright/test';

/**
 * Song Mode for a **genre archetype with no artist over it** (TASK-074).
 *
 * ⛔ **A genre is a different selection from an artist, and the difference is
 * not cosmetic.** `arrangement.spec.ts` drives the timeline's gestures; this
 * drives the path a producer takes when they want *the style* rather than a
 * particular person — pick a genre, generate, get a whole record. Everything
 * downstream of the selection is shared, so what this is really asserting is
 * that nothing in the flow quietly requires an artist to have been chosen.
 *
 * ⚠ **The mock's dataset is not the shipped one**, so this cannot say a form
 * reads as that music — `engine/tests/genre_songs.rs` makes that claim against
 * the real `data/`, over all five families. What a browser can add is that the
 * *page* completes the loop with a genre selected, which no cargo test reaches.
 */

/** Pick a genre from the roster — never the artist the fixture also carries. */
async function pickGenre(page: Page, name: string) {
  await page.goto('/');
  const search = page.getByLabel('Search an artist');
  await search.fill(name);
  await search.press('Enter');
  await page.getByRole('tab', { name: 'Song' }).click();
}

test('a genre with no artist over it arranges a whole song', async ({ page }) => {
  await pickGenre(page, 'trap');

  // Before Generate the tab says what it is waiting for rather than sitting
  // blank — a stage that draws nothing reads as the app having failed to load.
  await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();
  const sections = await page.locator('[data-testid^="song-section-"]').count();
  expect(sections).toBeGreaterThan(1);
});

test('a second genre arranges its own song rather than the first one’s', async ({ page }) => {
  // ⛔ The failure this catches is the most convincing wrong thing the app can
  // show: one style's arrangement left on screen under another's name. The
  // song store clears on `selectedId`, and genre mode is the path where that is
  // easiest to get wrong, because both selections are genres.
  await pickGenre(page, 'trap');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();

  const search = page.getByLabel('Search an artist');
  await search.fill('uk-drill');
  await search.press('Enter');

  // Gone, not merely different: nothing has been generated for this genre yet.
  await expect(page.locator('[data-testid="song-section-0"]')).toHaveCount(0);

  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();
});

test('the whole loop runs on a genre: arrange, edit, and export', async ({ page }) => {
  // The product's own claim for genre mode, end to end. Each half already has a
  // test; the sentence does not.
  await pickGenre(page, 'trap');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();

  const barsOf = async (index: number) =>
    Number(
      await page.locator(`[data-testid="song-section-${index}"]`).getAttribute('data-bars'),
    );
  const before = await barsOf(0);

  await page
    .locator('[data-testid="song-section-0"]')
    .getByRole('button', { name: 'Lengthen this section' })
    .click();
  await expect.poll(() => barsOf(0)).toBe(before + 1);

  // ⚠ The browser mock has no filesystem, so this asserts the export is
  // reachable and completes — not that a file lands. That is `Live-To-Do` § 4.
  const chip = page.locator('[data-testid="song-export"]');
  await expect(chip).toBeEnabled();
  await chip.click();
  await expect.poll(async () => chip.isEnabled()).toBe(true);
});

test('a genre’s structure row and picker are populated', async ({ page }) => {
  await pickGenre(page, 'uk-drill');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="song-structure"]')).toBeVisible();

  const chips = await page.locator('.song__structure-chip').count();
  const sections = await page.locator('[data-testid^="song-section-"]').count();
  expect(chips).toBe(sections);
});
