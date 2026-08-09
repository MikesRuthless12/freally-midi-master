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

test('a genre chip does not tell you it cannot be clicked', async ({ page }) => {
  // ⛔⛔ **`.chip` carried `cursor: not-allowed` from TASK-004 until 2026-08-09,
  // and Mike found it.** The layout shell drew these as inert placeholders;
  // TASK-028 wired every one to `select(genre.id)` and left the cursor behind.
  // So the product's only genre picker showed a "no entry" symbol over a control
  // that worked perfectly.
  //
  // ⚠ **The first test in this file was green for every one of those months**,
  // which is the point of this one: `click()` does not consult the cursor and
  // neither does the DOM, so a control can be fully wired, fully asserted, and
  // still be telling the producer it is dead. Only the computed style says what
  // they are actually being shown.
  const chip = page.locator('.chips .chip').first();
  await expect(chip).toBeVisible();
  await expect(chip).toHaveCSS('cursor', 'pointer');

  // ⚠ The other half of the same defect: the button has always set
  // `aria-pressed`, and nothing drew it — so the chips could not say which
  // genre was selected either. Compared against this chip's own idle border
  // rather than a sibling's, because selecting a genre cross-filters the row
  // and there may be no sibling left to compare with.
  const idle = await chip.evaluate((el) => getComputedStyle(el).borderColor);
  await chip.click();
  await expect(chip).toHaveAttribute('aria-pressed', 'true');
  const pressed = await chip.evaluate((el) => getComputedStyle(el).borderColor);
  expect(pressed).not.toBe(idle);
});
