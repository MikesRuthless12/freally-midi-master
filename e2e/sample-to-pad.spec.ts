import { expect, test, type Page } from '@playwright/test';

import { browserRow, openPanel } from './app';

/**
 * Browse → hear → drag → it stays there (TASK-058 / TASK-059A / TASK-054).
 *
 * ⛔⛔ **The gesture Mike named first, and the one nothing covered.** Every piece
 * of it had a test — `explorer::list` in Rust, the store's rules in
 * `explorer.test.ts`, the pad grid in `kit-panel.spec.ts` — and the *chain* had
 * none, so the suite could be green while the whole thing was broken end to end.
 * It was: `refuse_remote` classified the `\\?\` prefix that `canonicalize` puts
 * on every explorer path as a network path, and the drop was refused along with
 * the preview and the descent into subfolders.
 *
 * ⚠ **The mock had to learn to model a drop for this to be assertable at all.**
 * `explorer_drop` used to answer `undefined` and `kit_state` was a constant, so a
 * spec could perform the drag and then had nothing to check — the pad read the
 * same before and after. That is *why* this file did not exist. See the note on
 * `droppedSamples` in `ipc-mock.ts`.
 *
 * ⚠ Chromium, not WebView2. What this proves is the page's half of the chain:
 * that the row is a drag source, the pad is a drop target, the command carries
 * the right lane and path, and the pad's own label changes to say so. That the
 * *physical* drag works inside a DAW's WebView2 is `Live-To-Do.md`'s row and
 * needs a human.
 */

/** The pad holding `lane`, by the lane's own data attribute rather than by text. */
function pad(page: Page, lane: string) {
  return page.locator(`.pad[data-lane="${lane}"]`);
}

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  // ⛔ The panel is behind a vertical tab now — `openPanel` presses it.
  await openPanel(page, 'explorer');
});

test('a sample dragged from the browser lands on the pad and stays there', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();

  const source = browserRow(page, 'kick-hard.wav');
  const target = pad(page, 'snare');

  // ⚠ The snare, deliberately — the fixture's *kick* already carries a sample,
  // so dropping there could pass while changing nothing.
  await expect(target).toHaveAttribute('data-assigned', 'false');

  await source.dragTo(target);

  // ⛔ **The pad's own label, which is the only thing that says the drop
  // landed.** Asserting that the command fired would pass on a plugin that
  // refused it — which is exactly what was happening.
  await expect(target).toHaveAttribute('data-assigned', 'true');
  await expect(target.locator('.pad__source')).toHaveText('kick-hard.wav');
});

test('a drop replaces what the pad was already holding', async ({ page }) => {
  await browserRow(page, 'Samples').click();

  const kick = pad(page, 'kick');
  // The fixture's kick starts on a producer's own sample.
  await expect(kick.locator('.pad__source')).toHaveText('my-kick.wav');

  await browserRow(page, 'kick-808.wav').dragTo(kick);

  await expect(kick.locator('.pad__source')).toHaveText('kick-808.wav');
});

/**
 * ⛔ **The second route, and both must land identically** (TASK-059A).
 *
 * *"Or select it and press the slot's own button … This is the path that works
 * without a mouse, and the one that works when the explorer and the pad grid are
 * not on screen together."* Both routes end in the same command, so a sample
 * assigned either way is byte-identical on the pad.
 */
test('a selected sample can be put on a pad without dragging', async ({ page }) => {
  await browserRow(page, 'Samples').click();

  // Nothing selected yet, so no pad offers to use one.
  await expect(
    pad(page, 'perc').getByRole('button', { name: /^Put the selected/ }),
  ).toBeHidden();

  await browserRow(page, 'kick-808.wav').click();
  await pad(page, 'perc')
    .getByRole('button', { name: /^Put the selected/ })
    .click();

  await expect(pad(page, 'perc')).toHaveAttribute('data-assigned', 'true');
  await expect(pad(page, 'perc').locator('.pad__source')).toHaveText('kick-808.wav');
});

test('both routes produce the same pad state', async ({ page }) => {
  await browserRow(page, 'Samples').click();

  // Route one: drag.
  await browserRow(page, 'clap-01.wav').dragTo(pad(page, 'snare'));
  // Route two: select and press.
  await browserRow(page, 'clap-01.wav').click();
  await pad(page, 'rim')
    .getByRole('button', { name: /^Put the selected/ })
    .click();

  await expect(pad(page, 'snare').locator('.pad__source')).toHaveText('clap-01.wav');
  await expect(pad(page, 'rim').locator('.pad__source')).toHaveText('clap-01.wav');
});

test('a .mid offers no pad button, because it cannot be a one-shot', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'riff.mid').click();
  // ⛔ The same rule the two drag MIME types enforce, on the keyboard route.
  await expect(
    pad(page, 'perc').getByRole('button', { name: /^Put the selected/ }),
  ).toBeHidden();
});

test('a folder is not draggable, because nothing could mean', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  // ⛔ There is nothing a folder could do when dropped on one drum lane, so it
  // is not offered — an affordance that can only fail is worse than none.
  await expect(browserRow(page, 'Kicks')).toHaveAttribute('draggable', 'false');
  await expect(browserRow(page, 'kick-808.wav')).toHaveAttribute('draggable', 'true');
});

test('clearing a pad puts the shipped sound back', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  const target = pad(page, 'clap');
  await browserRow(page, 'clap-01.wav').dragTo(target);
  await expect(target.locator('.pad__source')).toHaveText('clap-01.wav');

  // ⚠ The clear control is only drawn when there is something to clear, so a pad
  // on its built-in sound has no control that would do nothing.
  await target.getByRole('button', { name: /^Clear/ }).click();
  await expect(target).toHaveAttribute('data-assigned', 'false');
});

test('the preview player draws the sample the tree selected', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();
  await browserRow(page, 'kick-hard.wav').click();

  // ⛔ The path itself, not merely that an `<svg>` exists — a panel rendering an
  // empty box would pass a "is the waveform there" check while drawing nothing.
  await expect(page.locator('.preview__outline').first()).toHaveAttribute('d', /^M[\d.,L]+Z$/);
  await expect(browserRow(page, 'kick-hard.wav')).toHaveAttribute('aria-selected', 'true');
});
