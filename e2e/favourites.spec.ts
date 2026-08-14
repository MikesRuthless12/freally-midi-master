import { expect, test, type Page } from '@playwright/test';

import { browserRow, openPanel } from './app';

/**
 * Starred favourites (TASK-058C).
 *
 * Mike, 2026-08-10: *"if we star one, it adds it to a list that we can see so
 * that way we can click just on the name of that 'Starred Favorite' and it will
 * take us to the exact spot of that exact sample … and if it's not a folder that
 * you still have in the 'File Explorer' then it should take you there in Windows
 * Explorer or the macOS Explorer, so all 'Starred Favorites' should be able to be
 * reachable at any given time."*
 *
 * ⚠ The OS half cannot happen in a browser, so the mock records the *request*
 * rather than pretending a window opened — `window.__freallyRevealed`, the same
 * shape the sample-copy gate uses. Actually launching Explorer is a
 * `Live-To-Do.md` row.
 */

/**
 * ⚠ **Every locator here is scoped to the tree or to the list, never the page.**
 * A starred file carries the label "Unstar <name>" in *both* — that is the point
 * of the feature, since a favourite whose folder is shut has no tree row to
 * unstar it from — so an unscoped locator is ambiguous by design rather than by
 * accident.
 */
function tree(page: Page) {
  return page.getByRole('tree', { name: 'Sample list' });
}

function list(page: Page) {
  return page.getByRole('list', { name: 'Starred favourites' });
}

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  // ⛔ The panel is behind a vertical tab now — `openPanel` presses it.
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
});

test('a star fills in and the file joins the list', async ({ page }) => {
  // ⛔ Nothing starred, so the list is absent rather than an empty heading.
  await expect(list(page)).toBeHidden();

  await tree(page).getByRole('button', { name: 'Star kick-808.wav' }).click();

  // ⛔ Mike named the two states: outline when unstarred, filled when starred.
  // `aria-pressed` is the state; the fill follows it in `FileTree`.
  await expect(tree(page).getByRole('button', { name: 'Unstar kick-808.wav' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );

  await expect(list(page)).toBeVisible();
  await expect(list(page).getByRole('listitem')).toHaveCount(1);
  await expect(list(page)).toContainText('kick-808.wav');
});

test('a .mid can be starred too, not only samples', async ({ page }) => {
  // ⛔ Mike: *"for samples/one shots/midi"*. A starred `.mid` has to be as
  // reachable as a starred one-shot.
  await tree(page).getByRole('button', { name: 'Star riff.mid' }).click();
  await expect(list(page)).toContainText('riff.mid');
});

test('folders have no star, because finding folders is what the tabs are for', async ({
  page,
}) => {
  await expect(tree(page).getByRole('button', { name: 'Star Kicks' })).toBeHidden();
});

test('unstarring forgets it, from the list as well as the tree', async ({ page }) => {
  await tree(page).getByRole('button', { name: 'Star kick-808.wav' }).click();
  await expect(list(page)).toBeVisible();

  // ⛔ Reachable from the list too: a favourite whose folder is closed has no row
  // in the tree to unstar it from.
  await list(page).getByRole('button', { name: 'Unstar kick-808.wav' }).click();

  await expect(list(page)).toBeHidden();
  await expect(tree(page).getByRole('button', { name: 'Star kick-808.wav' })).toHaveAttribute(
    'aria-pressed',
    'false',
  );
});

test('clicking a favourite reveals it in the tree, expanding the way down', async ({
  page,
}) => {
  // Star something two levels down, then shut the branch it lives in.
  await browserRow(page, 'Kicks').click();
  await tree(page).getByRole('button', { name: 'Star kick-hard.wav' }).click();
  await browserRow(page, 'Kicks').click();
  await expect(browserRow(page, 'kick-hard.wav')).toBeHidden();

  // ⚠ `.favourites__go`, not a name match: the row also holds an unstar button
  // labelled with the same filename.
  await list(page).locator('.favourites__go').filter({ hasText: 'kick-hard.wav' }).click();

  // ⛔ **The exact spot.** Expanding only the immediate parent would leave it
  // inside a branch that is still shut, so the row would never appear.
  await expect(browserRow(page, 'kick-hard.wav')).toBeVisible();
  await expect(browserRow(page, 'kick-hard.wav')).toHaveAttribute('aria-selected', 'true');
});

test('a favourite outside the open library is handed to the OS instead', async ({ page }) => {
  // ⛔ The half that makes *"reachable at any given time"* true. With only eight
  // folder tabs a producer will close the folder a favourite lives in, and the
  // panel cannot reveal what it is no longer browsing.
  await tree(page).getByRole('button', { name: 'Star kick-808.wav' }).click();
  await page.getByRole('button', { name: 'Remove Samples from the library' }).click();

  const outside = list(page).locator('.favourites__go').filter({ hasText: 'kick-808.wav' });
  await expect(outside).toBeVisible();
  await outside.click();

  // ⚠ The request, not a window: a browser cannot open Explorer, and a mock that
  // pretended it had would make a broken reveal look like a working one.
  await expect
    .poll(() => page.evaluate(() => window.__freallyRevealed ?? []))
    .toContain('/library/Samples/kick-808.wav');
});
