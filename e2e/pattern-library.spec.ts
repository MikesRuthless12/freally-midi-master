import { expect, test } from '@playwright/test';
import { openPanel, pickArtist, pickCombo } from './app';

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
  // ⛔ The panel is behind a vertical tab now — `openPanel` presses it.
  await openPanel(page, 'patterns');
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
  //
  // ⚠ Comboboxes since TASK-057, so the options live in a portalled listbox
  // that only exists while the field is open — a `<select>`'s were always in
  // the DOM.
  const parts = page.getByRole('combobox', { name: 'Filter by part' });
  await parts.click();
  const partOptions = page.getByRole('listbox', { name: 'Filter by part' }).getByRole('option');
  await expect(partOptions).toHaveCount(2);
  await expect(partOptions.nth(1)).toHaveText('Drums');
  await page.keyboard.press('Escape');

  const artists = page.getByRole('combobox', { name: 'Filter by artist' });
  await artists.click();
  await expect(
    page.getByRole('listbox', { name: 'Filter by artist' }).getByRole('option'),
  ).toHaveCount(2);
  await page.keyboard.press('Escape');

  // Choosing the one that is there keeps the row; going back to "all" keeps it
  // too, so the filter is a narrowing rather than a mode.
  await pickCombo(page, 'Filter by part', 'Drums');
  await expect(parts).toHaveValue('Drums');
  await expect(page.locator('.patterns__item')).toHaveCount(1);

  await pickCombo(page, 'Filter by part', 'All parts');
  await expect(parts).toHaveValue('All parts');
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
