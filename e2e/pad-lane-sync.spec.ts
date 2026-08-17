import { expect, test } from '@playwright/test';
import { pickArtist, pickCombo } from './app';

/**
 * The pads and the grid's rows name the same lanes (2026-08-16).
 *
 * ⛔⛔ **Mike found them disagreeing.** He pointed a pad at Mid tom and reported
 * *"it didn't change any to mid tom"*, then asked for both directions: *"ensure
 * that these in the drum lanes in the pattern generator are the same as the
 * one's in the drum pad's"* and *"vice-versa, if you change the one's in the
 * generator it should change the pad's names"*.
 *
 * ▶ **They looked identical and were not.** Both controls draw their names from
 * the same `lanes.*` keys, so nothing on screen hinted that the pad picker wrote
 * to the persisted pad layout while the row picker renamed the lane inside the
 * clip. The two only ever agreed because they happened to start that way.
 *
 * ⚠ **Only a browser can show this**, because the whole point is that two
 * components in different rails, reading different stores, end up saying the
 * same thing. `cells.test.ts` owns what `reassignLane` does to a pattern; this
 * owns whether the other control hears about it.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await pickArtist(page, 'Mock Artist');
  await page.getByRole('button', { name: 'Generate', exact: true }).first().click();
  await expect(page.locator('.grid__track').first()).toBeVisible();
});

/** The names down the grid's row headers, which is what a producer reads. */
/**
 * ⛔ **`textContent`, NOT `allInnerTexts()`.** `.grid__lanename` is
 * `text-transform: uppercase`, and `innerText` returns the *rendered* text — so
 * the tidier-looking call answers `["CLOSED HAT", "SNARE", "KICK"]` and every
 * assertion here compares against the model's own `Kick`. Tried during a
 * cleanup pass and reverted; `lane-mute.spec.ts` can use `allInnerTexts` on the
 * same locator only because it asserts the uppercase form.
 */
const laneNames = (page: import('@playwright/test').Page) =>
  page
    .locator('.grid__lanename')
    .evaluateAll((nodes) => nodes.map((node) => (node.textContent ?? '').trim()));

test('pointing a pad at another drum renames that row in the generator', async ({ page }) => {
  // Pad 1 holds Kick, and the generated clip has a Kick row — the case Mike hit.
  await expect(page.getByRole('combobox', { name: 'Pad 1 lane' })).toHaveValue('Kick');
  expect(await laneNames(page)).toContain('Kick');

  await pickCombo(page, 'Pad 1 lane', 'Mid tom');

  await expect(page.getByRole('combobox', { name: 'Pad 1 lane' })).toHaveValue('Mid tom');
  await expect.poll(() => laneNames(page)).toContain('Mid tom');
  expect(await laneNames(page)).not.toContain('Kick');
});

test('reassigning a row in the generator renames the pad that plays it', async ({ page }) => {
  // ⛔ The other direction, which is the half that had no route at all: the row
  // picker rewrote the clip and left the pad pointing at a lane that no longer
  // existed.
  await expect(page.getByRole('combobox', { name: 'Pad 2 lane' })).toHaveValue('Snare');

  // ⚠ **Located by its row, not by its accessible name** — and the first cut of
  // this test failed for a reason worth keeping: the picker is named
  // *"Change {lane} to another drum"*, so the rename this test is asserting also
  // renames the control being driven. `pickCombo` lost its own element between
  // `Enter` and `blur`. The row is the stable handle.
  const snareRow = page
    .locator('.grid__row')
    .filter({ has: page.locator('.grid__lanename', { hasText: /^Snare$/ }) });
  const picker = snareRow.locator('.grid__slot input');
  await picker.click();
  await picker.fill('Ride');
  await picker.press('Enter');

  await expect(page.getByRole('combobox', { name: 'Pad 2 lane' })).toHaveValue('Ride');
  await expect.poll(() => laneNames(page)).toContain('Ride');
});

test('a pad pointed at a drum the clip already has leaves the beat alone', async ({ page }) => {
  // ⛔⛔ **The guard, and the reason this is safe to ship.** `reassignLane`
  // refuses a target the pattern already holds, because merging two lanes' hits
  // into one would silently delete part of the beat. The pad still moves — that
  // is the layering case the pads have always allowed — and the clip does not.
  const before = await laneNames(page);
  expect(before).toContain('Kick');
  expect(before).toContain('Snare');

  await pickCombo(page, 'Pad 1 lane', 'Snare');

  await expect(page.getByRole('combobox', { name: 'Pad 1 lane' })).toHaveValue('Snare');
  expect(await laneNames(page)).toEqual(before);
});
