import { expect, test } from '@playwright/test';

/**
 * The sample browser and its audition player, end to end (TASK-132).
 *
 * ⛔ **This is the gate for the class of defect this panel *was*.** Every one of
 * `explorer_state`, `explorer_waveform`, `explorer_drop` and the seven
 * `preview_*` commands was written, tested from Rust, and called by nothing —
 * ten commands and two modules of dead code, invisible to every gate in the repo
 * because each one asks "does what is wired up work" and nothing asked "is it
 * wired up". `src/state/explorer.test.ts` covers the store's rules; what only a
 * browser shows is that the panel is actually reading them.
 *
 * ⚠ The mock has no filesystem and no audio thread, so the position never
 * advances and no dialog can open. What can be checked here is the wiring and
 * the shape: the library, the listing, the selection, the waveform being drawn
 * from real peaks, and the rail actually resizing. Hearing a sample is
 * `Live-To-Do.md`'s half and needs a human in a DAW.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('lists the saved library and the folder inside it', async ({ page }) => {
  // The root the mock reports, named rather than shown as a full path — a
  // producer recognises "Samples" far faster than "D:\Audio\Libraries\Samples".
  await expect(page.getByRole('button', { name: 'Samples', exact: true })).toBeVisible();

  const list = page.getByRole('list', { name: 'Sample list' });
  await expect(list).toBeVisible();
  // Folders sort first, then the audio files — the order `explorer::list`
  // returns and the order every DAW browser uses.
  await expect(list.getByRole('listitem')).toHaveText(['Kicks', 'clap-01.wav', 'kick-808.wav']);
});

test('"up" is disabled at a root, which is the containment boundary showing', async ({
  page,
}) => {
  // ⛔ Not cosmetic. `Explorer::state` nulls the parent at a root so browsing
  // cannot walk out of the folder the producer added, up through their home
  // directory, and enumerate the disk from inside a plugin. The mock reports a
  // null parent for exactly that reason, and this is the page half of it.
  await expect(page.getByRole('button', { name: 'Up' })).toBeDisabled();
});

test('clicking a sample draws its waveform and offers the transport', async ({ page }) => {
  await expect(page.getByText('Pick a sample to preview it.')).toBeVisible();

  await page.getByRole('button', { name: 'kick-808.wav' }).click();

  // ⛔ **The path itself, not merely that an `<svg>` exists.** A panel that
  // rendered an empty box would pass a "is the waveform there" check while
  // drawing nothing — which is the readout-that-lies failure this whole panel
  // was written after. `d` comes from the peaks the command returned.
  const outline = page.locator('.preview__outline').first();
  await expect(outline).toHaveAttribute('d', /^M[\d.,L]+Z$/);

  // ⚠ **Scoped to the preview bar, and it has to be.** Play, Stop and Loop are
  // also the *pattern* transport's labels up in the stage header — deliberately,
  // because they mean the same thing and `PreviewPlayer` reuses the translated
  // `transport.*` strings rather than inventing a second set. An unscoped
  // locator matches both and tells you nothing about either.
  const bar = page.locator('.preview__bar');
  for (const label of ['Play', 'Stop', 'Loop', 'Play backwards']) {
    await expect(bar.getByRole('button', { name: label, exact: true })).toBeVisible();
  }
  // "Playback time out of total time", verbatim from Mike's spec.
  await expect(page.locator('.preview__time')).toHaveText('0:00.0 / 0:01.5');
});

test('the loop and reverse toggles report their own state', async ({ page }) => {
  await page.getByRole('button', { name: 'kick-808.wav' }).click();

  const bar = page.locator('.preview__bar');
  const loop = bar.getByRole('button', { name: 'Loop', exact: true });
  await expect(loop).toHaveAttribute('aria-pressed', 'false');
  await loop.click();
  // ⚠ Written through immediately rather than waiting for the poll: the plugin
  // holds the authority, but a producer pressing a toggle has to see it move on
  // the frame they pressed it.
  await expect(loop).toHaveAttribute('aria-pressed', 'true');

  const reverse = bar.getByRole('button', { name: 'Play backwards', exact: true });
  await expect(reverse).toHaveAttribute('aria-pressed', 'false');
  await reverse.click();
  await expect(reverse).toHaveAttribute('aria-pressed', 'true');
});

test('the rail widens as it is dragged, and the stage gives up the pixels', async ({
  page,
}) => {
  // ⛔ **Both halves, because Mike asked for both.** 2026-08-07: *"the whole
  // file explorer panel is able to be resized so that you can see long file
  // names and that the center panel shrinks as you expand file explorer, but
  // don't let it get absurdly wide."*
  const rail = page.locator('.rail--left');
  const stage = page.locator('.stage');
  const before = (await rail.boundingBox())!;
  const stageBefore = (await stage.boundingBox())!;

  const handle = page.locator('.rail__resizer');
  const grip = (await handle.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + 120, grip.y + grip.height / 2, { steps: 8 });
  await page.mouse.up();

  const after = (await rail.boundingBox())!;
  const stageAfter = (await stage.boundingBox())!;
  expect(after.width).toBeGreaterThan(before.width);
  expect(stageAfter.width).toBeLessThan(stageBefore.width);
});

test('it cannot be dragged absurdly wide', async ({ page }) => {
  // The ceiling Mike named. Dragged to the far side of the window, the rail
  // stops at `RAIL_MAX_WIDTH` rather than swallowing the stage.
  const handle = page.locator('.rail__resizer');
  const grip = (await handle.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + 4000, grip.y + grip.height / 2, { steps: 10 });
  await page.mouse.up();

  const after = (await page.locator('.rail--left').boundingBox())!;
  expect(after.width).toBeLessThanOrEqual(560);
  // ...and the stage is still a usable size rather than a sliver.
  expect((await page.locator('.stage').boundingBox())!.width).toBeGreaterThan(300);
});
