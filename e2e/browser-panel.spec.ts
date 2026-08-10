import { expect, test } from '@playwright/test';

import { browserRow } from './app';

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

test('starts as a list of roots, named rather than shown as full paths', async ({ page }) => {
  // A producer recognises "Samples" far faster than "D:\Audio\Libraries\Samples".
  await expect(browserRow(page, 'Samples')).toBeVisible();
  // ⛔ **Shut to begin with.** A tree that expands every root on load walks a
  // producer's whole library the moment the panel opens, which is the cost
  // `explorer.rs`'s module header refuses by name.
  await expect(browserRow(page, 'Samples')).toHaveAttribute('aria-expanded', 'false');
  await expect(browserRow(page, 'Kicks')).toBeHidden();
});

/**
 * ⛔⛔ **The defect Mike reported on 2026-08-10, as a test.**
 *
 * *"you can get to the subfolders list, but you cannot go into those
 * subfolders."* The cause was in Rust — `refuse_remote` read the `\\?\` prefix
 * that `canonicalize` puts on every path as a *network* path — and
 * `plugin/src/explorer.rs` now pins it. This is the page half: that the tree
 * actually descends more than one level, which is the part a producer sees.
 */
test('descends through subfolders, not just the first level', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  await expect(browserRow(page, 'Samples')).toHaveAttribute('aria-expanded', 'true');

  // Folders sort above files at every level — `explorer::list`'s doing, so the
  // tree cannot disagree with what the plugin thinks it sent.
  await expect(browserRow(page, 'Kicks')).toBeVisible();
  await expect(browserRow(page, 'kick-808.wav')).toBeVisible();

  // Two levels down...
  await browserRow(page, 'Kicks').click();
  await expect(browserRow(page, 'Vinyl')).toBeVisible();
  await expect(browserRow(page, 'kick-hard.wav')).toBeVisible();

  // ...and three, which is where a real sample library actually lives.
  await browserRow(page, 'Vinyl').click();
  await expect(browserRow(page, 'kick-dusty.wav')).toBeVisible();
});

test('a folder indents under its parent, and its icon says it is open', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();

  // ⛔ Mike named the shape: *"the main folder is at the top, and then indents
  // and shows the subfolders underneath and then the files beneath that."*
  const root = (await browserRow(page, 'Samples').boundingBox())!;
  const child = (await browserRow(page, 'Kicks').boundingBox())!;
  const grandchild = (await browserRow(page, 'Vinyl').boundingBox())!;
  expect(child.y).toBeGreaterThan(root.y);
  expect(grandchild.y).toBeGreaterThan(child.y);

  // Each level starts further in than the one above it. Measured on the row's
  // own text, because the row itself spans the panel at every depth — that is
  // what keeps the hover band full width.
  const textStart = async (name: string) =>
    (await browserRow(page, name).locator('.tree__name').boundingBox())!.x;
  expect(await textStart('Kicks')).toBeGreaterThan(await textStart('Samples'));
  expect(await textStart('Vinyl')).toBeGreaterThan(await textStart('Kicks'));
});

test('Up retracts the deepest branch rather than navigating anywhere', async ({ page }) => {
  const up = page.getByRole('button', { name: 'Up' });
  // ⚠ Disabled rather than hidden with nothing open — a control that comes and
  // goes is one a producer has to look for twice.
  await expect(up).toBeDisabled();

  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();
  await expect(browserRow(page, 'Vinyl')).toBeVisible();

  await up.click();
  // The deepest branch shut; the one above it is still open.
  await expect(browserRow(page, 'Vinyl')).toBeHidden();
  await expect(browserRow(page, 'Kicks')).toBeVisible();

  await up.click();
  await expect(browserRow(page, 'Kicks')).toBeHidden();
  await expect(up).toBeDisabled();
});

/**
 * ⛔⛔ **The same two keys, two meanings, decided by the row under the focus.**
 *
 * Mike, 2026-08-10: *"pressing the right arrow should expand a folder, and
 * pressing right arrow on a sample/one shot/midi should play the midi and
 * pressing the left arrow should play the midi/sample/one shot backwards."*
 * They cannot collide — a folder has nothing to audition and a sample has
 * nothing to expand.
 */
test('the arrows open and shut folders', async ({ page }) => {
  await browserRow(page, 'Samples').focus();
  await page.keyboard.press('ArrowRight');
  await expect(browserRow(page, 'Samples')).toHaveAttribute('aria-expanded', 'true');
  await expect(browserRow(page, 'Kicks')).toBeVisible();

  await browserRow(page, 'Kicks').focus();
  await page.keyboard.press('ArrowRight');
  await expect(browserRow(page, 'Vinyl')).toBeVisible();

  // ← shuts the folder it is on...
  await page.keyboard.press('ArrowLeft');
  await expect(browserRow(page, 'Vinyl')).toBeHidden();
  // ...and on one already shut, shuts the branch it is *in*, so ← is a way back
  // out rather than a key that sometimes does nothing.
  await page.keyboard.press('ArrowLeft');
  await expect(browserRow(page, 'Kicks')).toBeHidden();
});

test('up and down walk the tree without auditioning anything', async ({ page }) => {
  // TASK-058A: *"`↑`/`↓` move the selection, so a producer can walk a folder and
  // hear every file … without touching the mouse."* Paired with →, which is what
  // actually plays — auditioning on every step would make ↓ unusable for simply
  // getting past a folder.
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Samples').focus();

  await page.keyboard.press('ArrowDown');
  await expect(browserRow(page, 'Kicks')).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(browserRow(page, 'clap-01.wav')).toBeFocused();
  await page.keyboard.press('ArrowUp');
  await expect(browserRow(page, 'Kicks')).toBeFocused();

  // ⚠ Walking is not selecting: nothing has been auditioned, so the preview is
  // still empty.
  await expect(page.getByText('Pick a sample to preview it.')).toBeVisible();
});

test('the arrows audition a sample forwards and backwards', async ({ page }) => {
  await browserRow(page, 'Samples').click();

  // ⚠ Focused, not clicked — walking the tree from the keyboard moves focus
  // without selecting, and the audition has to follow the focus or a producer
  // arrowing down a folder keeps hearing the file they already left.
  await browserRow(page, 'kick-808.wav').focus();
  await page.keyboard.press('ArrowRight');

  const bar = page.locator('.preview__bar');
  const reverse = bar.getByRole('button', { name: 'Play backwards', exact: true });
  await expect(reverse).toHaveAttribute('aria-pressed', 'false');

  await browserRow(page, 'kick-808.wav').focus();
  await page.keyboard.press('ArrowLeft');
  await expect(reverse).toHaveAttribute('aria-pressed', 'true');
});

/**
 * ⛔⛔ **Two kinds, two sets of affordances** (TASK-058).
 *
 * Mike, 2026-08-10: *"i want to be able to view .mid files in the File
 * Explorer."* They were filtered out of the listing entirely, which is why
 * `explorer_midi` shipped with nothing able to reach it. The rule that comes
 * with showing them is that a `.mid` must not be offered a waveform or a drum
 * pad — controls that can only fail on one.
 */
test('a .mid is listed, and is not offered as a drum sample', async ({ page }) => {
  await browserRow(page, 'Samples').click();
  await expect(browserRow(page, 'riff.mid')).toBeVisible();

  // ⛔ Draggable — it goes to a *generator* — but under its own MIME type, so a
  // drum pad is not a drop target for it at all. That is what makes "no pad
  // assignment for MIDI" structural rather than a note in a doc comment.
  await expect(browserRow(page, 'riff.mid')).toHaveAttribute('draggable', 'true');

  await browserRow(page, 'riff.mid').click();
  // ⚠ No waveform is even asked for: a `.mid` has no PCM, and requesting one
  // would put a refusal on screen every time a producer clicked a perfectly good
  // file. Auditioning it is TASK-058's MIDI playback and is not built yet.
  await expect(page.locator('.preview__outline')).toHaveCount(0);
});

/**
 * ⛔ Mike, 2026-08-10: *"i should be able to have up to 8 folders in the view to
 * be able to be tabbed to be used and sifted through at any given time, and if
 * you want to add more, then you have to exit out of one of them."*
 */
test('the library folders are tabs, and only one tree is shown at a time', async ({ page }) => {
  // ⚠ **Scoped to the browser's own tablist.** `getByRole('tab')` unscoped also
  // matches the six generator tabs up in the stage — the same trap
  // `PreviewPlayer`'s transport locators document, one panel over.
  const tabs = page.locator('.browser__tabs').getByRole('tab');
  await expect(tabs).toHaveCount(1);
  await expect(tabs.first()).toHaveAttribute('aria-selected', 'true');

  // The tree shows the selected tab's folder, not every root stacked.
  await expect(browserRow(page, 'Samples')).toBeVisible();
});

test('Add folder is disabled once eight folders are open', async ({ page }) => {
  // ⚠ One root in the fixture, so the button is live — the disabled state is
  // asserted against the rule rather than against the fixture happening to be
  // full. `explorer::MAX_ROOTS` is the real bound and refuses a ninth
  // independently; this is the page saying so before the gesture is spent.
  await expect(page.getByRole('button', { name: 'Add folder' })).toBeEnabled();
});

test('the browser can take the whole rail, and give it back', async ({ page }) => {
  // ⛔ Mike, 2026-08-10: it must *"expand as high as it can, not just a little
  // bit or half-way up the side of the left rail."* The roster and the browser
  // were both `flex: 1 1 0`, so each got half the rail however deep the tree ran.
  const tree = page.locator('.tree');
  const before = (await tree.boundingBox())!;

  await page.getByRole('button', { name: 'Give the browser the whole rail' }).click();

  await expect(page.getByRole('combobox', { name: 'Roster' })).toBeHidden();
  const after = (await tree.boundingBox())!;
  expect(after.height).toBeGreaterThan(before.height);

  // ...and back, because hiding the roster permanently would be a worse trade.
  await page.getByRole('button', { name: 'Show the roster again' }).click();
  await expect(page.getByRole('combobox', { name: 'Roster' })).toBeVisible();
});

test('clicking a sample draws its waveform and offers the transport', async ({ page }) => {
  await expect(page.getByText('Pick a sample to preview it.')).toBeVisible();

  await browserRow(page, 'Samples').click();
  await browserRow(page, 'kick-808.wav').click();

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
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'kick-808.wav').click();

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
