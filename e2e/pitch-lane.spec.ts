import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

/**
 * A drum row expanding into a pitch lane (TASK-161).
 *
 * `src/components/DrumGrid/cells.test.ts` owns the bucketing — that the window
 * is seven rows, that hits land in the row their pitch names, that a hit past
 * the window is reported rather than dropped, and that a drag onto an occupied
 * pitch refuses instead of eating the note underneath. `src/state/editing.ts`
 * owns the view state.
 *
 * What only a browser shows is that the chevron is on every lane and that
 * opening one actually adds rows to the grid. Mike, 2026-08-12: *"i want all
 * the lanes to have the ability to expand into a pitch lane"* — a disclosure
 * wired to one lane would pass every test above this one.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  await pickArtist(page, 'Mock Artist');
  await page.getByRole('button', { name: 'Generate', exact: true }).first().click();
  await expect(page.locator('.grid__track').first()).toBeVisible();
});

test('every lane offers the pitch chevron, and they all start collapsed', async ({ page }) => {
  const chevrons = page.locator('.grid__expand');
  const rows = page.locator('.grid__row:not(.grid__row--pitch)');

  await expect(chevrons).toHaveCount(await rows.count());
  await expect(page.locator('.grid__expand[aria-expanded="true"]')).toHaveCount(0);
  await expect(page.locator('.grid__row--pitch')).toHaveCount(0);
});

test('opening a lane adds exactly seven pitch rows, and closing takes them away', async ({
  page,
}) => {
  const chevron = page.locator('.grid__expand:not([disabled])').first();

  await chevron.click();
  await expect(chevron).toHaveAttribute('aria-expanded', 'true');
  await expect(page.locator('.grid__row--pitch')).toHaveCount(7);
  // Exactly one of the seven is the lane's own root.
  await expect(page.locator('.grid__row--pitch[data-root]')).toHaveCount(1);

  await chevron.click();
  await expect(chevron).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('.grid__row--pitch')).toHaveCount(0);
});

test('two lanes open at once keep their own windows', async ({ page }) => {
  // ⚠ The point of a per-lane offset: opening a second lane must not close or
  // move the first, which one shared window would have done.
  const chevrons = page.locator('.grid__expand:not([disabled])');

  await chevrons.nth(0).click();
  await chevrons.nth(1).click();

  await expect(page.locator('.grid__expand[aria-expanded="true"]')).toHaveCount(2);
  await expect(page.locator('.grid__row--pitch')).toHaveCount(14);
});

test('dragging a hit between rows moves it — it does not duplicate or delete it', async ({
  page,
}) => {
  // ⛔⛔ The regression this guards, and it went both ways. A drag that begins
  // and ends inside one cell fires a trailing `click`, and that click ran the
  // place/clear toggle on the hit that had just been dragged: on a refused drag
  // it DELETED the note the refusal existed to protect, and on a successful one
  // it placed a second hit on the pitch the note had just left.
  const chevron = page.locator('.grid__expand:not([disabled])').first();
  await chevron.click();

  const rows = page.locator('.grid__row--pitch');
  const lit = page.locator('.grid__row--pitch .grid__cell--on');
  const before = await lit.count();
  expect(before).toBeGreaterThan(0);

  // Row-to-row distance, which is what one semitone costs the pointer.
  const first = (await rows.nth(0).boundingBox())!;
  const second = (await rows.nth(1).boundingBox())!;
  const step = second.y - first.y;

  const box = (await lit.first().boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;

  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y - step, { steps: 8 });
  await page.mouse.up();

  // Exactly as many hits as before: one moved, none added, none lost.
  await expect(lit).toHaveCount(before);
});

test('a pitch cell places a hit on its own row and leaves the others alone', async ({
  page,
}) => {
  const chevron = page.locator('.grid__expand:not([disabled])').first();
  await chevron.click();

  const row = page.locator('.grid__row--pitch').first();
  const cell = row.locator('.grid__cell').nth(5);
  const before = await page.locator('.grid__row--pitch .grid__cell--on').count();

  await cell.click();

  await expect(cell).toHaveClass(/grid__cell--on/);
  expect(await page.locator('.grid__row--pitch .grid__cell--on').count()).toBe(before + 1);
});
