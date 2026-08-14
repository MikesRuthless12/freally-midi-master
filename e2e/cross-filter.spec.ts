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
 * ⛔ **The cross-filtering survived into the roster box's OPEN LIST only**
 * (2026-08-12). It was dropped from the comboboxes entirely for a while, because
 * a combobox is narrowed by *typing* and hiding entries only stops them being
 * found — with the rail selecting an artist on load, a filtered genre box meant
 * "UK Drill" could not be typed at all. That rule was wider than its defect: what
 * must stay whole is what a **query** reaches. Mike, looking at the rail:
 * *"changing to another genre doesn't change the actual names in the Roster to
 * names within that Genre."* So the roster's browse list follows the selected
 * genre, its `filter` still searches everything, and the genre box is untouched.
 * `e2e/genre-browse.spec.ts` owns the narrowing; this file owns what is left
 * whole.
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
  // exists** — the five-hundred-row roster. ⚠ The roster combobox narrows again
  // since 2026-08-12, and this stays removed: the box says which genre is
  // selected in its own field, so a notice repeating it under the blurb is the
  // running-together string Mike asked to be rid of and nothing more.
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

test('the genre box keeps offering every genre, whatever is selected', async ({ page }) => {
  // ⛔⛔ **Load-bearing, and the half the 2026-08-12 narrowing deliberately did
  // not touch.** Selecting an artist used to hide every genre they do not work
  // in. A producer with an artist selected could then not type their way to any
  // other genre — so the genre box is never narrowed by anything, and the roster
  // box narrows only what it *lists*, never what its `filter` can find.
  await pickArtist(page, 'Mock Artist');

  const genres = page.getByRole('combobox', { name: 'Genres' });
  await genres.click();
  await genres.fill('UK');
  await expect(page.locator('.combo__menu').getByRole('option').first()).toContainText(
    'UK Drill',
  );
});
