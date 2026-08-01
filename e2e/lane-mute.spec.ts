import { expect, test } from '@playwright/test';

/**
 * The per-lane preview mute (FMM-S02).
 *
 * `plugin/src/shared.rs` owns the semantics — that a mute is a bit in a mask
 * the audio thread reads without a lock, and that `render_preview` skips a
 * muted lane's notes. `src/state/session.test.ts` owns the store rules: the set
 * is sorted, an unmute that empties it is expressible, and a no-op click
 * records nothing.
 *
 * What only a browser shows is that the control is on the lane it claims to be
 * on. A mute wired to the wrong row silences the wrong drum, and every test
 * above this one would still pass.
 *
 * ⛔ The mock has no audio thread, so nothing here asserts anything went quiet.
 * The wiring is what this can prove, and the wiring is what was missing — the
 * Rust half shipped complete, with nothing in the UI able to set it.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  await page.locator('.roster__item', { hasText: 'Mock Artist' }).click();
  await page
    .getByRole('button', { name: /generate/i })
    .first()
    .click();
  await expect(page.locator('.grid__track').first()).toBeVisible();
});

test('every lane offers a mute, and it starts unmuted', async ({ page }) => {
  const mutes = page.locator('.grid__mute');
  const rows = page.locator('.grid__row');
  await expect(mutes).toHaveCount(await rows.count());

  for (const mute of await mutes.all()) {
    await expect(mute).toHaveAttribute('aria-pressed', 'false');
  }
});

test('muting a lane marks that row and leaves the others alone', async ({ page }) => {
  const kick = page.locator('.grid__row').first();
  const snare = page.locator('.grid__row').nth(1);

  await kick.locator('.grid__mute').click();

  await expect(kick.locator('.grid__mute')).toHaveAttribute('aria-pressed', 'true');
  await expect(kick).toHaveAttribute('data-muted', 'true');
  // The row next door must be untouched. A mask written with the wrong bit
  // shift would silence a neighbour, and both rows have the same markup.
  await expect(snare.locator('.grid__mute')).toHaveAttribute('aria-pressed', 'false');
  await expect(snare).not.toHaveAttribute('data-muted', 'true');
});

test('the label says it is the preview, and does not change with the state', async ({
  page,
}) => {
  // ⛔ Two rules at once. The notes are already on the host's track by the time
  // the sampler runs, so a label reading "Mute kick" would promise something
  // this control does not do — the wording is the feature.
  //
  // ⛔ And the name must stay put across a toggle, because `aria-pressed` is
  // what carries the state. Swapping to "Unmute…" *and* setting `aria-pressed`
  // announces "Unmute kick in the preview, toggle button, pressed", which
  // contradicts itself and leaves the listener unable to tell what is muted.
  const mute = page.locator('.grid__row').first().locator('.grid__mute');
  const before = await mute.getAttribute('aria-label');
  expect(before).toMatch(/preview/i);

  await mute.click();
  await expect(mute).toHaveAttribute('aria-pressed', 'true');
  await expect(mute).toHaveAttribute('aria-label', before ?? '');
});

test('a muted lane keeps its notes on screen', async ({ page }) => {
  // Dimmed, not emptied: the pattern did not change, only what we play of it.
  const kick = page.locator('.grid__row').first();
  const before = await kick.locator('.grid__cell--on').count();
  expect(before).toBeGreaterThan(0);

  await kick.locator('.grid__mute').click();
  await expect(kick.locator('.grid__cell--on')).toHaveCount(before);
});
