import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

/**
 * The pattern library (TASK-045A).
 *
 * `plugin/src/patterns.rs` owns the store — the round trip, the traversal
 * boundary, the corrupt-file skip, the atomic write. What only a browser shows
 * is the gesture: generate something, name it, and find it again — and that
 * loading one puts its notes back on screen.
 *
 * ⛔ The browser mock keeps its library for the lifetime of the page rather
 * than reporting a fake success, because "save it and it is in the list" is the
 * whole feature and a fixture that pretended would make a broken save look like
 * a working one.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await pickArtist(page, 'Mock Artist');
  await page.getByRole('button', { name: 'Generate', exact: true }).first().click();
  await expect(page.locator('.grid__track').first()).toBeVisible();
});

test('a saved pattern appears in the library with what it is', async ({ page }) => {
  const name = page.getByLabel('Name this pattern');
  await name.fill('Keeper');
  await page.locator('.patterns').getByRole('button', { name: 'Save', exact: true }).click();

  const row = page.locator('.patterns__item', { hasText: 'Keeper' });
  await expect(row).toHaveCount(1);
  // ⛔ The row says what it is without opening it — artist, bars and tempo —
  // because choosing between twenty saved loops is the thing the list is for.
  await expect(row.locator('.patterns__meta')).toHaveText(/bars/);
  // And the mini grid is drawn from the summary rather than from the notes.
  await expect(row.locator('.patterns__step')).toHaveCount(32);

  await expect(name).toHaveValue('', 'the box empties, so the next save is not a duplicate');
});

test('loading a saved pattern puts its notes back on screen', async ({ page }) => {
  const cells = () =>
    page
      .locator('.grid__row')
      .first()
      .locator('.grid__cell')
      .evaluateAll((els) => els.map((e) => e.getAttribute('data-hits')));

  await page.getByLabel('Name this pattern').fill('Keeper');
  await page.locator('.patterns').getByRole('button', { name: 'Save', exact: true }).click();
  const saved = await cells();

  // Generate over the top of it, then load the saved one back.
  await page.getByRole('button', { name: 'Generate', exact: true }).first().click();
  await page
    .locator('.patterns__item', { hasText: 'Keeper' })
    .locator('.patterns__load')
    .click();

  await expect.poll(cells).toEqual(saved);
});

test('the filters offer only what the library actually holds', async ({ page }) => {
  await page.getByLabel('Name this pattern').fill('Drums One');
  await page.locator('.patterns').getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.locator('.patterns__item')).toHaveCount(1);

  // ⛔ **The options are derived from the library, not from the tab list.** A
  // filter offering "Bassline" over a library with no basslines in it is a
  // control that can only ever produce an empty list — the producer clicks it,
  // sees nothing, and cannot tell a working filter from a lost pattern.
  const parts = page.getByLabel('Filter by part');
  await expect(parts.locator('option')).toHaveCount(2);
  await expect(parts.locator('option').nth(1)).toHaveText('Drums');

  const artists = page.getByLabel('Filter by artist');
  await expect(artists.locator('option')).toHaveCount(2);

  // Choosing the one that is there keeps the row; going back to "all" keeps it
  // too, so the filter is a narrowing rather than a mode.
  await parts.selectOption({ index: 1 });
  await expect(page.locator('.patterns__item')).toHaveCount(1);
  await parts.selectOption('');
  await expect(page.locator('.patterns__item')).toHaveCount(1);
});

test('a pattern can be deleted, and Save is refused with no name', async ({ page }) => {
  const save = page.locator('.patterns').getByRole('button', { name: 'Save', exact: true });
  // ⛔ Disabled rather than saving an unnamed row the producer could not find
  // again.
  await expect(save).toBeDisabled();

  await page.getByLabel('Name this pattern').fill('Temporary');
  await save.click();
  const row = page.locator('.patterns__item', { hasText: 'Temporary' });
  await expect(row).toHaveCount(1);

  await row.getByRole('button', { name: 'Delete Temporary' }).click();
  await expect(row).toHaveCount(0);
});
