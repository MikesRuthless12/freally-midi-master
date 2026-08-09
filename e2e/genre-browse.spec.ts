import { expect, test } from '@playwright/test';

/**
 * Genre browse UX (TASK-047, FR-009).
 *
 * The roadmap's verify line is *"filter by 'drill' lists drill artists +
 * genres"*, and the rest is what makes a roster of 500 browsable without
 * pressing Generate on each one: a badge that says what a row *is*, and a pane
 * that says what the selection tends to do.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('a genre chip narrows the roster to that genre, and says it has', async ({ page }) => {
  // ⚠ Scoped to the genre chips' own container: `.chip` is also the seed
  // chip's class, on the other side of the window.
  const chips = page.locator('.chips .chip');
  await expect(chips.first()).toBeVisible();
  const name = (await chips.first().textContent())?.trim() ?? '';

  await chips.first().click();

  // ⛔ **Narrowed *and* labelled.** A list that lost half itself with nothing
  // saying so reads as a broken dataset rather than as a filter.
  await expect(page.locator('.roster__filter')).toContainText(name);
  await expect(page.locator('.roster__item[aria-pressed="true"]')).toContainText(name);

  // And there is a way back out that is not "reload".
  await page.getByRole('button', { name: 'Show all' }).click();
  await expect(page.locator('.roster__filter')).toHaveCount(0);
});

test('a row says whether it is an artist or a genre', async ({ page }) => {
  // ⛔ The badge earns its place in *search results*, where the two kinds are
  // mixed with no headings — but it has to be on the row, and this is where a
  // row can be found deterministically.
  const genres = page.locator('.roster__group', { hasText: 'Genres' });
  await expect(genres.locator('.badge--genre').first()).toHaveText('Genre');

  const artists = page.locator('.roster__group', { hasText: 'Artists' });
  await expect(artists.locator('.badge--genre')).toHaveCount(0);
});

test('the pane says what the selection tends to do, before you generate', async ({ page }) => {
  // ⛔ Nothing selected is *no pane*, not an empty one — a heading over blanks
  // reads as something that failed to load.
  await expect(page.locator('.artistpane')).toHaveCount(0);

  await page.locator('.roster__item', { hasText: 'Mock Artist' }).click();

  const pane = page.locator('.artistpane');
  await expect(pane).toBeVisible();
  await expect(pane.locator('.artistpane__name')).toContainText('Mock Artist');
  // ⚠ The tempo comes from the *model's* defaults, not from the pins — a pane
  // showing the pins would describe an artist who plays neither.
  await expect(pane.locator('.artistpane__tends')).toContainText('BPM');

  // And it follows the selection rather than sticking to the first thing read.
  const genre = page.locator('.roster__group', { hasText: 'Genres' }).locator('.roster__item');
  const genreName = (await genre.first().locator('.roster__name').textContent())?.trim() ?? '';
  await genre.first().click();
  await expect(pane.locator('.artistpane__name')).toContainText(genreName);
});
