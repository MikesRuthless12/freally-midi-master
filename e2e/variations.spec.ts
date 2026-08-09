import { expect, test } from '@playwright/test';

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
  await page.locator('.roster__item', { hasText: 'Mock Artist' }).click();
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
