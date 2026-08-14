import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

/**
 * The variation history (TASK-045).
 *
 * `src/state/variations.test.ts` owns the log's rules — no cap, per-part
 * counters, the resolved tempo rather than the pinned one, append-only
 * branching. What only a browser shows is that ◀ and ▶ are wired to it and
 * that stepping back puts a whole setup back rather than just a number.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await pickArtist(page, 'Mock Artist');
});

test('the counter starts empty and counts every generation of this part', async ({ page }) => {
  const count = page.locator('.variations__count');
  await expect(count).toHaveText('No takes yet');

  const generate = page.getByRole('button', { name: 'Generate', exact: true }).first();
  await generate.click();
  await expect(count).toHaveText('1 / 1');
  await generate.click();
  await expect(count).toHaveText('2 / 2');
  await generate.click();
  await expect(count).toHaveText('3 / 3');
});

test('◀ steps back through the takes and ▶ comes forward again', async ({ page }) => {
  const generate = page.getByRole('button', { name: 'Generate', exact: true }).first();
  await generate.click();
  await generate.click();
  await generate.click();
  const count = page.locator('.variations__count');
  await expect(count).toHaveText('3 / 3');

  const back = page.getByRole('button', { name: 'Previous generation' });
  const forward = page.getByRole('button', { name: 'Next generation' });

  // ⛔ At the newest take there is nowhere forward to go, and the control says
  // so rather than wrapping silently.
  await expect(forward).toBeDisabled();

  await back.click();
  await expect(count).toHaveText('2 / 3');
  await back.click();
  await expect(count).toHaveText('1 / 3');
  await expect(back).toBeDisabled();

  await forward.click();
  await expect(count).toHaveText('2 / 3');
});

test('stepping back pins the seed that made that take', async ({ page }) => {
  // ⛔ **The seed is *pinned* on the way back, and that is the load-bearing
  // half.** `generate` sends `null` unless the seed is pinned — the fix for
  // "Generate returns the same beat every press" — so a recall that restored
  // the number without pinning it would draw a fresh seed on the next press and
  // land somewhere the producer has never been.
  //
  // ⚠ Asserted through the *lock*, not the number: the browser mock answers
  // with one fixed seed, so comparing values here would prove nothing.
  const generate = page.getByRole('button', { name: 'Generate', exact: true }).first();
  const lock = page.getByRole('button', { name: /the seed/ });

  await generate.click();
  await generate.click();
  // An echoed seed is not a pinned one — that distinction is the whole reason
  // Generate stopped returning the same beat forever.
  await expect(lock).toHaveAttribute('aria-pressed', 'false');

  await page.getByRole('button', { name: 'Previous generation' }).click();
  await expect(lock).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.seed__input')).not.toHaveValue('');
});

test('each generator counts its own takes', async ({ page }) => {
  // Rerolling one part advances that part and nothing else; a single global
  // number would claim the chords changed when they did not.
  const generate = page.getByRole('button', { name: 'Generate', exact: true }).first();
  const count = page.locator('.variations__count');

  await generate.click();
  await generate.click();
  await expect(count).toHaveText('2 / 2');

  await page.getByRole('tab', { name: 'Melody' }).click();
  await expect(count).toHaveText('No takes yet');
  await page.getByRole('button', { name: 'Generate', exact: true }).first().click();
  await expect(count).toHaveText('1 / 1');

  await page.getByRole('tab', { name: 'Drums' }).click();
  await expect(count).toHaveText('2 / 2');
});

/**
 * Browsing the history rather than stepping through it (TASK-045B).
 *
 * ⛔⛔ **Mike said why the arrows are not enough**: *"if you have generated 20
 * just 'Trap' and 20 just 'Rage' and 40 just 'Drake' then it should persist …
 * so that way you can go through the actual history of all your generations and
 * find what you like."* ◀/▶ answer "back one"; finding take twelve out of eighty
 * needs an index, and this is it.
 *
 * ⚠ `src/state/variations.test.ts` owns the round trip through the plugin and
 * `plugin/src/takes.rs` owns the cap and the per-style eviction. What only a
 * browser shows is that the counter opens the panel, that the panel lists what
 * was generated, and that choosing a take puts it back.
 */
test('the counter opens a browsable history of every take', async ({ page }) => {
  const generate = page.getByRole('button', { name: 'Generate', exact: true }).first();
  await generate.click();
  await generate.click();
  await generate.click();

  // ⛔ The way in is the readout a producer is already looking at when they
  // decide they want an earlier take back.
  await page.locator('.variations__count').click();
  const history = page.getByRole('dialog', { name: 'Take history' });
  await expect(history).toBeVisible();

  // Grouped by style, which is the grouping in Mike's own sentence.
  await expect(history.locator('.takes__style')).toHaveText([/Mock Artist/]);
  await expect(history.locator('.takes__take')).toHaveCount(3);
  // Enough to choose between them: which generator, how long, how fast, and
  // when — a list of three identical rows would be no better than the arrows.
  await expect(history.locator('.takes__take').first()).toContainText('Drums');
  await expect(history.locator('.takes__take').first()).toContainText('bars');
});

test('choosing a take from the history puts it back and shuts the panel', async ({ page }) => {
  const generate = page.getByRole('button', { name: 'Generate', exact: true }).first();
  await generate.click();
  await generate.click();
  await generate.click();
  const count = page.locator('.variations__count');
  await expect(count).toHaveText('3 / 3');

  await count.click();
  const history = page.getByRole('dialog', { name: 'Take history' });
  // Newest first here, oldest first on disk — the last row is the first take.
  await history.locator('.takes__take').last().click();

  // ⛔ **Shut on choosing.** It is a place you go, not a place you live: leaving
  // it open over the grid would cover the thing the take was chosen *for*.
  await expect(history).toBeHidden();
  // ⚠ The counter follows, because recalling walks the same log the arrows do.
  await expect(count).toHaveText('1 / 3');
});

test('the history can be emptied, and says so when it is', async ({ page }) => {
  // ⚠ It is a record of what somebody has been making. Being able to clear it is
  // not a convenience — the browser's history carries the same sentence.
  const generate = page.getByRole('button', { name: 'Generate', exact: true }).first();
  await generate.click();

  await page.locator('.variations__count').click();
  const history = page.getByRole('dialog', { name: 'Take history' });
  await expect(history.locator('.takes__take')).toHaveCount(1);

  await history.getByRole('button', { name: 'Clear take history' }).click();
  await expect(history.getByText('Nothing generated yet.')).toBeVisible();
});
