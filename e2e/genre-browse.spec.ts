import { expect, test } from '@playwright/test';
import { pickArtist, pickGenre } from './app';

/**
 * Genre and artist browsing (TASK-047, FR-009).
 *
 * ⛔⛔ **The chips and the roster list are gone** (2026-08-09). The rail was a
 * search box, a six-chip genre row and a five-hundred-row list; it is now a
 * genre combobox and a roster combobox with the details pane under them. Mike:
 * *"we won't need the search textbox … it will save us some room."*
 *
 * The roadmap's verify line survives the change and is what this still proves:
 * *"filter by 'drill' lists drill artists + genres"* — one box finds both kinds,
 * and a row says which kind it is.
 *
 * ⚠ **The old `cursor: not-allowed` test went with the chips it guarded.** Its
 * lesson did not: a control can be fully wired, fully asserted and still be
 * telling the producer it is dead, because `click()` consults neither the cursor
 * nor the computed style. If a picker ever grows a disabled-looking state again,
 * that assertion is worth writing back.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('one box finds artists and genres alike, and says which is which', async ({ page }) => {
  // ⛔ **Both kinds in one list, which is what the roster held.** Splitting them
  // across two boxes would have meant a producer had to know whether "UK Drill"
  // was an artist or a genre before they could find it.
  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await roster.fill('uk');

  const options = page.locator('.combo__menu').getByRole('option');
  await expect(options.first()).toContainText('UK Drill');
  // The badge is what tells the two kinds apart, now that there are no headings
  // above them.
  await expect(options.first().locator('.combo__badge')).toHaveText('Genre');
});

test('picking a genre leaves the details clean of a filter notice', async ({ page }) => {
  // ⛔⛔ **INVERTED 2026-08-11.** This read `toContainText('Trap')` against
  // `.roster__filter`, and then pressed "Show all" for the way back. Mike:
  // *"I also don't want the 'Filtered by DrakeShow all' to show up at all in the
  // details part of the roster."* `e2e/cross-filter.spec.ts` carries the full
  // reasoning; this is the second door onto the same removed control and is kept
  // as a refusal so reinstating it fails in both places.
  await pickGenre(page, 'Trap');

  await expect(page.locator('.roster__filter')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Show all' })).toHaveCount(0);
});

test('the pane says what the selection tends to do, before you generate', async ({ page }) => {
  await pickArtist(page, 'Mock Artist');

  const pane = page.locator('.artistpane');
  await expect(pane).toBeVisible();
  await expect(pane.locator('.artistpane__name')).toContainText('Mock Artist');
  // ⚠ The tempo comes from the *model's* defaults, not from the pins — a pane
  // showing the pins would describe an artist who plays neither.
  await expect(pane.locator('.artistpane__tends')).toContainText('BPM');
});

test('the pane follows the selection rather than sticking to the first thing read', async ({
  page,
}) => {
  await pickArtist(page, 'Mock Artist');
  await expect(page.locator('.artistpane__name')).toContainText('Mock Artist');

  await pickGenre(page, 'Trap');
  await expect(page.locator('.artistpane__name')).toContainText('Trap');
});

test('“Original Workflow” is reachable whatever genre is selected', async ({ page }) => {
  // ⛔⛔ **Mike, 2026-08-09**: *"ensure that 'Original Workflow' is at the top of
  // the artist/producer combobox no matter which genre is selected, so that way
  // you can always start an original artist/producer workflow and save it."* It
  // is the way in to building your own, so no filter may hide it.
  await pickGenre(page, 'UK Drill');

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();

  const options = page.locator('.combo__menu').getByRole('option');
  await expect(options.first()).toContainText('Original Workflow');
});

test('choosing Original Workflow opens the editor rather than selecting a style', async ({
  page,
}) => {
  // ⛔ It is an *action*, not a choice. Nothing may land on it by accident —
  // when it was an ordinary option the never-blank fallback chose it, blur
  // committed it, and the style editor opened over the whole app.
  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await page.locator('.combo__menu').getByRole('option').first().click();

  await expect(page.getByRole('dialog', { name: 'Style editor' })).toBeVisible();
});
