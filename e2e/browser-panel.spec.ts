import { expect, test } from '@playwright/test';

import { browserRow, openPanel } from './app';

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
  // ⛔ The panel is behind a vertical tab now — `openPanel` presses it.
  await openPanel(page, 'explorer');
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

test('up and down walk the tree, and a sample auditions as it is reached', async ({ page }) => {
  // ⛔⛔ **THIS ASSERTED THE OPPOSITE UNTIL 2026-08-11**, and the old rule was
  // TASK-058A's: *"↑/↓ move the selection … paired with →, which is what actually
  // plays — auditioning on every step would make ↓ unusable for simply getting
  // past a folder."* Mike overruled it by name: *"the files need to play as you
  // go up and down in the list with the up/down arrow or by clicking on them."*
  //
  // ⚠ **The folder case is what the old rule was protecting, and it still holds:**
  // walking onto a *directory* auditions nothing, because a directory has no
  // sample. So ↓ past a folder is still silent; it is only files that sound.
  await browserRow(page, 'Samples').click();
  // The listing arrives from the bridge, so the rows below Samples only exist a
  // tick later — arrowing before then walks a one-row tree.
  await expect(browserRow(page, 'Kicks')).toBeVisible();
  await browserRow(page, 'Samples').focus();

  await page.keyboard.press('ArrowDown');
  await expect(browserRow(page, 'Kicks')).toBeFocused();
  // A folder: nothing to hear, so the preview is still empty.
  await expect(page.getByText('Pick a sample to preview it.')).toBeVisible();

  await page.keyboard.press('ArrowDown');
  await expect(browserRow(page, 'clap-01.wav')).toBeFocused();
  // A file: reaching it selects it, and selecting is what auditions. The
  // waveform itself is the click test's business one screen down; what this one
  // pins is that walking moves the *selection* rather than only the highlight,
  // which is what the old behaviour did not do.
  await expect(browserRow(page, 'clap-01.wav')).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByText('Pick a sample to preview it.')).toBeHidden();

  await page.keyboard.press('ArrowUp');
  await expect(browserRow(page, 'Kicks')).toBeFocused();
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
  // file. ⛔ It can now be *heard* (TASK-160) — but through a render made on the
  // press, not through peaks drawn from a file that has none.
  await expect(page.locator('.preview__outline')).toHaveCount(0);
});

/**
 * Hearing a `.mid` from the browser (TASK-160).
 *
 * ⛔⛔ Mike, 2026-08-10: *".mid files … have its own sound like Ableton does that
 * can play the .mid file"*.
 *
 * ▶ **The roadmap costed this as "its own note scheduler" and it did not need
 * one.** `midi_audition::render` builds the same `Vec<f32>` a decoded `.wav`
 * arrives as, so the audition voice plays a MIDI file with the transport that
 * already exists — and `plugin/src/midi_audition.rs` owns the render's rules.
 * What only a browser shows is that the panel offers the controls and asks for
 * the render on the press rather than on the click.
 *
 * ⚠ The mock has no audio thread, so the position never advances. What is
 * checkable here is the wiring.
 */
test('a .mid can be played, and is not rendered until it is asked for', async ({ page }) => {
  await browserRow(page, 'Samples').click();

  await browserRow(page, 'riff.mid').click();
  // ⛔ **Selecting does not render**, and the disabled controls are how that is
  // visible: Stop and Loop act on a buffer, and there is not one yet. Building
  // the audio is the slow half, and walking a folder with ↓ would render every
  // `.mid` stepped past.
  const transport = page.locator('.midi__transport');
  await expect(transport).toBeVisible();
  await expect(transport.getByRole('button', { name: 'Stop' })).toBeDisabled();

  // ...and pressing Play is what asks for it, after which the rest of the
  // transport is live.
  await transport.getByRole('button', { name: 'Play' }).click();
  await expect(transport.getByRole('button', { name: 'Stop' })).toBeEnabled();
  await expect(transport.getByRole('button', { name: 'Loop' })).toBeEnabled();
  // The readout appears once something is loaded, rather than sitting at
  // `0:00 / 0:00` over a file nothing has rendered.
  await expect(transport.locator('.preview__time')).toBeVisible();
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

/**
 * ⛔⛔ **`the browser can take the whole rail, and give it back` was deleted on
 * 2026-08-12, and what it was gating is now structural.**
 *
 * It covered a "Give the browser the whole rail" button that hid the roster, for
 * Mike's 2026-08-10 request that the tree *"expand as high as it can, not just a
 * little bit or half-way up the side of the left rail"* — the roster and the
 * browser were both `flex: 1 1 0` and each got half the rail however deep the
 * tree ran.
 *
 * ▶ **The rail groups replaced the button with the layout.** `explorer` is a
 * group of its own (`RAIL_GROUPS` in `state/ui.ts`), so opening the browser
 * *already* takes the whole rail and the roster is not merely hidden — it is not
 * in that group. There is no button left to press and nothing to give back, and
 * the `explorer.fillRail` / `explorer.restoreRail` strings went with it.
 *
 * ⚠ The test below is what remains worth asserting: that the group really does
 * hand the browser the full height rather than a slot.
 */
test('the browser gets the whole rail, because it is a group of its own', async ({ page }) => {
  const rail = page.locator('.rail--left');
  const section = page.locator('.rail__section[data-section="explorer"]');

  await expect(page.getByRole('combobox', { name: 'Roster' })).toBeHidden();
  const railBox = (await rail.boundingBox())!;
  const sectionBox = (await section.boundingBox())!;
  // The one panel in the group, so it is the rail minus its own padding rather
  // than a half of it.
  expect(sectionBox.height).toBeGreaterThan(railBox.height * 0.8);
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
  for (const label of ['Stop', 'Loop', 'Play backwards']) {
    await expect(bar.getByRole('button', { name: label, exact: true })).toBeVisible();
  }
  // ⛔⛔ **`Pause`, not `Play`, and that is the point** — Mike, 2026-08-11: *"the
  // files need to play as you go up and down in the list with the up/down arrow
  // or by clicking on them."* Clicking the row auditions it, so by the time this
  // runs the one button that is both has already flipped. Asserting `Play` here
  // is what the old, silent-on-click behaviour looked like.
  await expect(bar.getByRole('button', { name: 'Pause', exact: true })).toBeVisible();
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

test('the waveform seeks where it is clicked, and keeps up while it is dragged', async ({
  page,
}) => {
  // TASK-058B's remaining half. The click was already there; dragging along the
  // waveform to hunt for a transient — the tape-rub every sample browser has —
  // was not, so the pointer had to be lifted and put down for every guess.
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'kick-808.wav').click();

  const wave = page.locator('.preview__wave');
  const box = (await wave.boundingBox())!;
  const time = page.locator('.preview__time');
  await expect(time).toHaveText('0:00.0 / 0:01.5');

  // Press at 60% of the width: 60% of a 1.5s sample is ~0.9s.
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height / 2);
  await page.mouse.down();
  await expect(time).toHaveText('0:00.9 / 0:01.5');

  // ⛔ Still held: the position has to follow the finger rather than waiting for
  // it to be lifted, which is the whole difference between a scrub and a click.
  await page.mouse.move(box.x + box.width * 0.2, box.y + box.height / 2);
  await expect(time).toHaveText('0:00.3 / 0:01.5');

  await page.mouse.up();
  await expect(time).toHaveText('0:00.3 / 0:01.5');
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

/**
 * The browser remembers what you opened (TASK-058).
 *
 * ⛔⛔ **Mike, 2026-08-12**: *"the history needs to persist with the new versions
 * and so does the file explorer's folder's list."* The folder list already did —
 * `library.json` sits in `%APPDATA%\Freally MIDI Master`, with no version segment
 * anywhere in the path — and there was **no history at all**, in the plugin or on
 * the page, to make persist. `plugin/src/recent.rs` writes into that same
 * directory so it inherits the property rather than being given its own; the
 * per-user path is pinned by a Rust test, and this pins the behaviour.
 */
test('auditioning a sample puts it at the top of the history', async ({ page }) => {
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();

  await browserRow(page, 'kick-808.wav').click();
  const history = page.getByRole('list', { name: 'Recent' });
  await expect(history.getByRole('listitem')).toHaveText([/kick-808\.wav/]);

  // ⛔ **Newest first, and one entry per file.** A history that appends a
  // duplicate every time you audition the same kick is a history you cannot
  // read — so re-opening promotes rather than repeats.
  await browserRow(page, 'clap-01.wav').click();
  await expect(history.getByRole('listitem')).toHaveText([/clap-01\.wav/, /kick-808\.wav/]);

  await browserRow(page, 'kick-808.wav').click();
  await expect(history.getByRole('listitem')).toHaveText([/kick-808\.wav/, /clap-01\.wav/]);
});

/**
 * ⛔⛔ **Opening a `.mid` counts as opening a file, and it did not.**
 *
 * The last security review found the page calling `loadRecent()` after
 * `explorer_midi_split` under a comment saying the plugin had written the
 * entry — and `recent::note` ran only from `preview_load` (a sample) and
 * `explorer_midi` (the *drop* into a part). So clicking twenty loops and
 * importing one gave a history of the one, which is `recent.rs`'s own recording
 * rule — *"recorded on audition, not on drop"* — backwards.
 *
 * ⚠ A separate test rather than another click in the one above, because the two
 * kinds go down two different commands: a sample records from `preview_load`
 * and a `.mid` has no PCM to load at all.
 */
test('opening a MIDI file records it too', async ({ page }) => {
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();

  await browserRow(page, 'riff.mid').click();
  const history = page.getByRole('list', { name: 'Recent' });
  await expect(history.getByRole('listitem')).toHaveText([/riff\.mid/]);

  // And it takes its place in one list with the samples, newest first — the
  // history is of files opened, not of two histories that happen to be adjacent.
  await browserRow(page, 'kick-808.wav').click();
  await expect(history.getByRole('listitem')).toHaveText([/kick-808\.wav/, /riff\.mid/]);
});

/**
 * A real sample library, at the size the plugin actually sends (TASK-058).
 *
 * ⛔⛔ **Mike's bound, verbatim: *"a 2,000-file folder under 300 ms"*.** 2,000 is
 * `explorer::MAX_ENTRIES` — the most rows the plugin will ever answer for one
 * folder — and the tree that shipped before this drew every one of them, as six
 * elements each, inside nested `<ul>`s built by a component that called itself.
 * Several folders can be open at once.
 *
 * ⚠ **The row count is the assertion that cannot be faked.** A timing bound alone
 * would pass on a fast machine with the old tree; the DOM holding thirty rows out
 * of two thousand is what proves the window exists.
 */
test('a two-thousand-file folder draws a window, not two thousand rows', async ({ page }) => {
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();

  // ⚠ **A smoke alarm, not the proof.** Mike's bound is 300 ms, but wall-clock
  // measured across a click and an auto-retrying assertion is as much a
  // measurement of the CI runner's afternoon as of the code — so this is set an
  // order of magnitude above it, to catch a change that puts the whole folder
  // back rather than to police a hundred milliseconds. **The row count below is
  // what actually proves virtualization**, and it cannot flake.
  const started = Date.now();
  await browserRow(page, 'Loops').click();
  await expect(browserRow(page, 'loop-0000.wav')).toBeVisible();
  expect(Date.now() - started).toBeLessThan(3_000);

  // ⛔ The whole point: the rows on screen plus the overscan, not the folder.
  const drawn = await page.locator('.tree__row').count();
  expect(drawn).toBeGreaterThan(0);
  expect(drawn).toBeLessThan(200);

  // ⚠ **The scrollbar still measures the whole list**, because the spacers hold
  // the height the un-drawn rows would have taken. Without that the list would
  // look 30 rows long and there would be nothing to scroll to row 1,999 with.
  const height = await page.locator('.tree').evaluate((box) => box.scrollHeight);
  expect(height).toBeGreaterThan(2_000 * 20);
});

/**
 * Type-to-filter (TASK-058), and the honest statement of what it searched.
 *
 * ⛔ The filter narrows the folders that have been **read** — the plugin reads one
 * folder per call, because walking a whole library on the host's editor thread is
 * what `Explorer::list_one` refuses to do. A box that looked like it searched the
 * whole library while searching part of it is the readout-that-lies failure, so
 * the scope line is part of the feature rather than decoration.
 */
test('type-to-filter narrows the tree, and says what it searched', async ({ page }) => {
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();
  await browserRow(page, 'Loops').click();

  await page.getByRole('searchbox', { name: 'Filter by name' }).fill('loop-1234');
  await expect(browserRow(page, 'loop-1234.wav')).toBeVisible();
  await expect(browserRow(page, 'loop-0000.wav')).toHaveCount(0);
  // The folders that lead to a match survive, or there is no path to it on screen.
  await expect(browserRow(page, 'Loops')).toBeVisible();
  await expect(page.getByText('Searching the folders you have opened.')).toBeVisible();

  // ⚠ **The root stays when nothing matches.** An empty panel reads as the
  // library having gone rather than as the query being too narrow.
  await page.getByRole('searchbox', { name: 'Filter by name' }).fill('nothing-is-called-this');
  await expect(page.getByText('Nothing here matches.')).toBeVisible();
  await expect(browserRow(page, 'Samples')).toBeVisible();

  // Escape is the way back, and without it the only one is select-all-delete.
  await page.getByRole('searchbox', { name: 'Filter by name' }).press('Escape');
  await expect(browserRow(page, 'loop-0000.wav')).toBeVisible();
});

test('the history can be emptied', async ({ page }) => {
  // ⚠ It is a record of where somebody has been, so being able to clear it is
  // not a convenience.
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'kick-808.wav').click();

  const history = page.getByRole('list', { name: 'Recent' });
  await expect(history.getByRole('listitem')).toHaveCount(1);

  await page.getByRole('button', { name: 'Clear history' }).click();
  await expect(page.getByRole('list', { name: 'Recent' })).toHaveCount(0);
});

/**
 * Training a style on a `.mid` from the browser (TASK-040T).
 *
 * ⛔⛔ **The gesture this task waited on, and the reason it is an e2e.** The fit,
 * its anti-collapse rules, the SMF reader and the keep/train loop all shipped on
 * 2026-08-09 — and the reader was reachable *only over the bridge*, so a producer
 * could train on their own generations and not on their own files. Every piece
 * was individually tested and the door was missing, which is the same shape as
 * the ten explorer commands this file's header records: each gate asked "does
 * what is wired up work" and none asked "is it wired up".
 *
 * ⚠ **The count is the assertion, not the button's own state.** A toggle that
 * lights up and reaches nothing is exactly the failure above. `riff.mid` splits
 * into three parts in the mock, so the style editor must move from `0 / 30` to
 * `3 / 30` — the number `engine::fit::MIN_KEPT` is a floor on.
 */
test('a .mid can be kept to train on, and the style editor counts its parts', async ({
  page,
}) => {
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'riff.mid').click();

  const keep = page.getByRole('button', { name: 'Train on this' });
  await expect(keep).toHaveAttribute('aria-pressed', 'false');
  await keep.click();
  await expect(keep).toHaveAttribute('aria-pressed', 'true');

  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });
  await expect(dialog).toBeVisible();
  // Three parts out of one file, counted as three patterns — which is what the
  // engine's floor is a floor on.
  await expect(dialog.locator('.styleeditor__kept')).toHaveText('3 / 30 kept');

  // ...and it is undone where it was done, rather than only inside the dialog.
  await dialog.getByRole('button', { name: 'Close' }).click();
  await keep.click();
  await expect(keep).toHaveAttribute('aria-pressed', 'false');

  await page.getByRole('button', { name: /Original Workflow/ }).click();
  await expect(dialog.locator('.styleeditor__kept')).toHaveText('0 / 30 kept');
});
