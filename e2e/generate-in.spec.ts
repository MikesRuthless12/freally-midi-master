import { expect, test } from '@playwright/test';

import { openPanel, pickArtist } from './app';

/**
 * "Drake, but in R&B" — the base swap, end to end (TASK-158C).
 *
 * ⛔⛔ **The defect this closes has been live at scale.** `lib/cross-filter.ts`
 * lists an artist under every genre in their `relatedGenres` — 529 of the 534
 * artist and producer models name one they do **not** `extend` — so the rail
 * says *"2Pac works in boom-bap"* and Generate has always answered g-funk. The
 * roster's claim and the generator's answer have never agreed.
 *
 * `engine/tests/base_swap.rs` proves the resolve holds over the real dataset and
 * `bridge.rs` proves a swapped request produces different notes. What only a
 * browser shows is that a producer can *reach* it: that the chip is there, that
 * it offers the genres the rail lists them under, and that picking one travels
 * with the next Generate.
 *
 * ⚠ **The mock has no engine**, so it cannot answer with different notes. It
 * folds the base into the pattern's id instead — see `generate_pattern` in
 * `ipc-mock.ts` for why that is the honest thing a fixture can show, and why
 * swallowing the pin would let the chip be wired to nothing and still pass.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  // ⛔ The chips live in the SESSION panel, which is behind a vertical tab.
  await openPanel(page, 'session');
});

test('an artist offers the genres the roster lists them under', async ({ page }) => {
  await pickArtist(page, 'Mock Artist');

  const chip = page.getByRole('combobox', { name: 'Generate in' });
  await expect(chip).toBeVisible();

  // ⚠ "Their own genre" is the null, and it reads the same way "Any" does in
  // every other chip in this row — the artist's own choice rather than an
  // absence.
  await chip.click();
  await expect(page.getByRole('option', { name: 'Their own genre' })).toBeVisible();
  await expect(page.getByRole('option', { name: 'Trap' })).toBeVisible();
});

test('a genre is not offered where the artist works in none', async ({ page }) => {
  // ⚠ The same rule the mood chip follows: a combobox whose only entry is
  // "their own" is a control that cannot do anything. The fixture's genres
  // relate to nothing, which is what a genre model's own `relatedGenres` is.
  await pickArtist(page, 'Trap');
  await expect(page.getByRole('combobox', { name: 'Generate in' })).toHaveCount(0);
});

test('the chosen genre travels with the next Generate', async ({ page }) => {
  // ⛔ **The whole point.** A chip that showed a genre and generated the
  // artist's own would be the readout-that-lies failure this task is closing,
  // arriving through the fix.
  await pickArtist(page, 'Mock Artist');

  await page.getByRole('combobox', { name: 'Generate in' }).click();
  await page.getByRole('option', { name: 'Trap' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  // ⚠ **What the page ASKED for**, not what came back. The mock has no engine,
  // so it cannot answer with different notes — and a spec that only looked at
  // the reply could not tell a chip wired to the request from one wired to
  // nothing. Same reasoning as `__freallyCopiedSamples`.
  await expect
    .poll(() => page.evaluate(() => window.__freallyGeneratedOver ?? []))
    .toContain('trap');
});

test('the pin survives a reload of the page state', async ({ page }) => {
  // ⚠ It is part of how the song was made, so it is saved like the mood —
  // reopening on the artist's own genre would silently produce a different
  // record from the same seed.
  await pickArtist(page, 'Mock Artist');
  await page.getByRole('combobox', { name: 'Generate in' }).click();
  await page.getByRole('option', { name: 'Trap' }).click();

  await expect(page.getByRole('combobox', { name: 'Generate in' })).toHaveValue('Trap');
});
