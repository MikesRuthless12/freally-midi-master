import { expect, test } from '@playwright/test';
import { pickArtist, pickGenre } from './app';

/**
 * What a selection *implies*, now that nothing is hidden by it.
 *
 * ⛔⛔ **This spec used to test a list narrowing, and that list is gone**
 * (2026-08-09). The rail was a search box, a genre chip row and a
 * five-hundred-row roster; it is now two comboboxes with the details pane under
 * them. Mike: *"instead of listing the roster, can we just have a combobox … it
 * shows the details under it?"*
 *
 * ⛔ **And the cross-filtering deliberately did NOT survive into the
 * comboboxes.** Narrowing made sense for a list — it was the only way to make
 * five hundred rows scannable. A combobox is narrowed by *typing*, so hiding
 * entries only stops them being found: with the rail selecting an artist on
 * load, a filtered genre box meant "UK Drill" could not be typed at all. So the
 * boxes always offer the whole roster.
 *
 * What survives is the *readout*: the rail still says what the selection implies
 * and still offers the way back. `src/lib/cross-filter.test.ts` owns the rules
 * themselves; this proves the rail is wired to them.
 *
 * The mock roster is two genres and one artist related to one of them
 * (`src/lib/ipc-mock.ts`), so "UK Drill" is a real empty case rather than a
 * contrived one.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('picking a genre puts no filter notice in the details', async ({ page }) => {
  // ⛔⛔ **INVERTED 2026-08-11, and kept as a refusal so it cannot come back.**
  // This read `toContainText('Filtered by Trap')`. Mike, looking at the rail:
  // *"I also don't want the 'Filtered by DrakeShow all' to show up at all in the
  // details part of the roster."* The two strings ran together on screen because
  // the notice and its button sat side by side under the artist blurb — but the
  // reason it goes is that it described a filter on a **list that no longer
  // exists**. Both comboboxes deliberately offer the whole roster (the test
  // below pins that), so there was nothing on screen for it to be true of.
  await pickGenre(page, 'Trap');

  await expect(page.locator('.roster__filter')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Show all' })).toHaveCount(0);
});

test('a genre nobody works in says so rather than leaving the rail silent', async ({
  page,
}) => {
  await pickGenre(page, 'UK Drill');

  await expect(page.locator('.rail__hint')).toContainText('No artists work in UK Drill yet.');
});

test('an empty genre stays selected, with only the hint to show for it', async ({ page }) => {
  // ⚠ **What is left of "Show all clears the notice without changing the
  // selection".** The button and the notice are gone (see above); the half that
  // still matters is that an empty genre is a *selection* rather than an error
  // the rail bounces off — it keeps the choice and says why the roster looks
  // bare.
  await pickGenre(page, 'UK Drill');

  await expect(page.locator('.rail__hint')).toContainText('No artists work in UK Drill yet.');
  await expect(page.getByRole('combobox', { name: 'Genres' })).toHaveValue('UK Drill');
});

test('both comboboxes keep offering the whole roster, whatever is selected', async ({
  page,
}) => {
  // ⛔⛔ **The rule that replaced cross-filtering, and it is load-bearing.**
  // Selecting an artist used to hide every genre they do not work in. If the
  // combobox did that, a producer who had an artist selected could not type
  // their way to any other genre — and since the rail selects one on load, that
  // would be the state the app opens in.
  await pickArtist(page, 'Mock Artist');

  const genres = page.getByRole('combobox', { name: 'Genres' });
  await genres.click();
  await genres.fill('UK');
  await expect(page.locator('.combo__menu').getByRole('option').first()).toContainText(
    'UK Drill',
  );
});
