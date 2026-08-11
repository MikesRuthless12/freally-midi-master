import { expect, test, type Page } from '@playwright/test';

/**
 * The arrangement view (TASK-063A, TASK-063B).
 *
 * ⛔ **Every test here asserts the resulting *arrangement*, never that a
 * handler fired.** A clone that inserts in the wrong place still fires its
 * handler; a resize that leaves a gap still updates the width it was told to.
 * The section headers publish their kind and bar count as data attributes, and
 * that is what these read — the same trick the piano roll's spec uses, and for
 * the same reason.
 */

/** Every section on screen, in playing order. */
async function sections(page: Page) {
  return page.locator('[data-testid^="song-section-"]').evaluateAll((nodes) =>
    nodes.map((node) => ({
      kind: node.getAttribute('data-kind'),
      bars: Number(node.getAttribute('data-bars')),
      left: Math.round(Number.parseFloat((node as HTMLElement).style.left) || 0),
      width: Math.round(Number.parseFloat((node as HTMLElement).style.width) || 0),
    })),
  );
}

/** The bar numbers the ruler is currently printing. */
async function rulerLabels(page: Page) {
  return page
    .locator('[data-testid="song-ruler"] .song__bar-number')
    .evaluateAll((nodes) => nodes.map((n) => Number(n.textContent)));
}

/** Open Song Mode with an arrangement in it. */
async function openSong(page: Page) {
  await page.goto('/');
  const search = page.getByRole('combobox', { name: 'Roster' });
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('tab', { name: 'Song' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();
}

test('generating a song lays its sections out end to end', async ({ page }) => {
  await openSong(page);
  const laid = await sections(page);

  expect(laid.length).toBeGreaterThan(1);
  // ⛔ The invariant everything else rests on: no gap and no overlap. A gap is
  // silence the ruler does not draw and an overlap is two sections claiming one
  // bar — both survive a screenshot.
  let expectedLeft = 0;
  for (const section of laid) {
    expect(section.left).toBe(expectedLeft);
    expect(section.bars).toBeGreaterThanOrEqual(1);
    expectedLeft += section.width;
  }
});

test('the ruler carries bar numbers and timestamps', async ({ page }) => {
  await openSong(page);

  const labels = await rulerLabels(page);
  expect(labels.length).toBeGreaterThan(1);
  // One-based, because every DAW counts bars from 1 and a producer reading "0"
  // assumes the display is broken.
  expect(labels[0]).toBe(1);
  expect(labels).toEqual([...labels].sort((a, b) => a - b));

  // Timestamps sit beside the numbers — "is the chorus inside 60 seconds" is a
  // question bars alone cannot answer.
  const times = await page
    .locator('[data-testid="song-ruler"] .song__time')
    .evaluateAll((nodes) => nodes.map((n) => n.textContent ?? ''));
  expect(times.length).toBe(labels.length);
  for (const time of times) expect(time).toMatch(/^\d+:\d{2}$/);
});

test('zooming changes the drawn grid resolution', async ({ page }) => {
  // ⛔ The requirement the roadmap states outright, and the one a screenshot
  // cannot check: a *fixed* grid looks perfectly reasonable at exactly one zoom
  // level. Both the grid's own resolution and the ruler's label spacing have to
  // move with the zoom.
  //
  // The grid is painted as a repeating gradient rather than mounted as one
  // element per line, so it publishes its resolution as data attributes — the
  // same trick the piano roll's canvas uses, and for the same reason: a painted
  // surface has no geometry a test can count.
  await openSong(page);

  const grid = page.locator('.song__grid');
  const resolution = async () => ({
    bar: Number(await grid.getAttribute('data-bar-step')),
    beat: Number(await grid.getAttribute('data-beat-step')),
  });

  const wideLabels = await rulerLabels(page);
  const wide = await resolution();

  await page.getByRole('button', { name: 'Zoom in' }).click();
  await page.getByRole('button', { name: 'Zoom in' }).click();
  await page.getByRole('button', { name: 'Zoom in' }).click();

  const closeLabels = await rulerLabels(page);
  const close = await resolution();

  // Zoomed in, the grid subdivides: beats appear and/or bar lines stop thinning.
  expect(close.beat >= wide.beat && close.bar <= wide.bar).toBe(true);
  expect(close.beat + (wide.bar - close.bar)).toBeGreaterThan(wide.beat);
  // More room means more labels for the same song.
  expect(closeLabels.length).toBeGreaterThanOrEqual(wideLabels.length);

  // And back out again, so the control is not one-way.
  for (let i = 0; i < 5; i += 1) {
    await page.getByRole('button', { name: 'Zoom out' }).click();
  }
  const out = await resolution();
  expect(out.beat).toBeLessThanOrEqual(close.beat);
  expect(out.bar).toBeGreaterThanOrEqual(close.bar);
  expect((await rulerLabels(page)).length).toBeLessThan(closeLabels.length);
});

test('a clip can be selected, and selecting one does not select the row', async ({ page }) => {
  await openSong(page);

  const drums = page.locator('[data-testid="song-clip-drums"]');
  const first = drums.first();
  await first.click();

  await expect(first).toHaveAttribute('aria-pressed', 'true');
  // ⛔ A clip is identified by section *and* part together. Selecting the drums
  // in the verse must not light up the drums in the hook — the bug a part-only
  // comparison would have, and it is invisible until a delete removes both.
  await expect(drums.nth(1)).toHaveAttribute('aria-pressed', 'false');
});

test('deleting a selected clip leaves its section standing', async ({ page }) => {
  await openSong(page);

  const before = await sections(page);
  const drums = page.locator('[data-testid="song-clip-drums"]');
  const count = await drums.count();
  await drums.first().click();
  await page.keyboard.press('Delete');

  // The clip goes; the section does not. Removing the drums from a verse is not
  // removing the verse, and one gesture must not do the other.
  await expect(drums).toHaveCount(count - 1);
  expect(await sections(page)).toEqual(before);
});

test('a section can be lengthened and shortened, and the rest follows', async ({ page }) => {
  await openSong(page);
  const before = await sections(page);

  await page
    .locator('[data-testid="song-section-0"]')
    .getByRole('button', { name: 'Lengthen this section' })
    .click();

  const longer = await sections(page);
  expect(longer[0].bars).toBe(before[0].bars + 1);
  // Everything after it moved, rather than a gap opening.
  expect(longer[1].left).toBeGreaterThan(before[1].left);
  let expectedLeft = 0;
  for (const section of longer) {
    expect(section.left).toBe(expectedLeft);
    expectedLeft += section.width;
  }

  await page
    .locator('[data-testid="song-section-0"]')
    .getByRole('button', { name: 'Shorten this section' })
    .click();
  expect((await sections(page))[0].bars).toBe(before[0].bars);
});

test('a section clones in place, after the one it came from', async ({ page }) => {
  await openSong(page);
  const before = await sections(page);

  await page.locator('[data-testid="song-section-1"] .song__section-name').dblclick();

  const after = await sections(page);
  expect(after).toHaveLength(before.length + 1);
  // The copy sits directly after its original, not at the end.
  expect(after[2].kind).toBe(before[1].kind);
  expect(after[2].bars).toBe(before[1].bars);

  let expectedLeft = 0;
  for (const section of after) {
    expect(section.left).toBe(expectedLeft);
    expectedLeft += section.width;
  }
});

test('copy and paste puts a clip onto a section that did not have one', async ({ page }) => {
  // ⛔ The paste has to land somewhere it is *visible*. The obvious version of
  // this test copies a clip and pastes it straight back onto its own section,
  // which is a no-op — so it can only assert "nothing broke", and it passes
  // just as well when paste does nothing at all.
  //
  // The intro carries a melody and no drums in every shipped form, so the
  // selection is moved there between the copy and the paste and the drum row
  // gains a clip it did not have.
  await openSong(page);

  const drums = page.locator('[data-testid="song-clip-drums"]');
  const before = await drums.count();

  await drums.first().click();
  await page.keyboard.press('Control+c');

  // Anchor the paste on the intro by selecting its melody — the first clip in
  // the melody row, because the intro is the first section that has one.
  await page.locator('[data-testid="song-clip-melody"]').first().click();
  await page.keyboard.press('Control+v');

  await expect(drums).toHaveCount(before + 1);
});

test('the transitions the export contains are drawn', async ({ page }) => {
  // ⛔ TASK-066's drop-out and decay are fields `song_to_smf` reads, so they are
  // in the file a producer drags out. A view that did not draw them would leave
  // the last beats of a section silent with nothing on screen saying why.
  await openSong(page);
  await expect(page.locator('.song__dropout').first()).toBeAttached();
  await expect(page.locator('.song__decay').first()).toBeAttached();
});

test('undo steps the arrangement back, and leaves the session alone', async ({ page }) => {
  // ⛔ **The bug this was written for, and it was watched failing.** Ctrl+Z is
  // bound globally to `useSession`'s history. The arrangement used to be a
  // different document with no history at all, so on this tab the chord undid
  // *the session* — a seed keystroke, a pin, or a piano-roll edit made on
  // another tab — while the arrangement stayed exactly as it was. The producer
  // sees nothing happen here and finds the damage later, somewhere else.
  //
  // The arrangement is part of the same snapshot now (`history.ts`), so the
  // shortcut does the obvious thing. Both halves are asserted: the section
  // really goes back, **and** the seed does not move — a stack that stepped the
  // session as well would pass a test that only checked the first.
  await openSong(page);

  const seed = page.getByLabel(/^Seed/);
  await seed.fill('123456');
  await seed.press('Enter');
  const afterEdit = await seed.inputValue();

  const before = (await sections(page))[0];
  const timeline = page.locator('[data-testid="song-section-0"]');
  await timeline.click();
  // Grow the first section by one bar, then take it back.
  await page.getByRole('button', { name: 'Lengthen this section' }).first().click();
  await expect.poll(async () => (await sections(page))[0].bars).toBe(before.bars + 1);

  await page.keyboard.press('Control+z');

  await expect.poll(async () => (await sections(page))[0].bars).toBe(before.bars);
  expect(await seed.inputValue()).toBe(afterEdit);
});

test('choosing another artist clears the arrangement that was on screen', async ({ page }) => {
  // ⛔ **The failure this catches is the most convincing wrong thing the app can
  // show**: the previous artist's whole song, still drawn, now under a different
  // artist's name. `session.select` nulls the pattern for exactly this reason
  // and never touched the song store, so every edit and every export afterwards
  // operated on the artist the producer had moved away from.
  //
  // It is the same claim `magic-moment.spec.ts` makes for a pattern, which is
  // what made the gap easy to miss — that test passes either way.
  await openSong(page);
  expect((await sections(page)).length).toBeGreaterThan(1);

  const search = page.getByRole('combobox', { name: 'Roster' });
  await search.fill('uk');
  await search.press('Enter');

  await expect(page.locator('[data-testid="song-section-0"]')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeEnabled();
});

test('cut then paste puts the clip back rather than on the first section', async ({ page }) => {
  // ⛔ `cut()` empties the selection, and the paste target used to be read from
  // it — `selection[0]?.sectionIndex ?? 0` — so Ctrl+X followed by Ctrl+V
  // dropped the clips onto the *intro*, a section they had never been in, and
  // left the one they came from empty.
  await openSong(page);

  const drums = page.locator('[data-testid="song-clip-drums"]');
  const before = await drums.count();

  // The last drums clip, so landing on section 0 would be unmistakable.
  await drums.last().click();
  await page.keyboard.press('Control+x');
  await expect(drums).toHaveCount(before - 1);

  await page.keyboard.press('Control+v');
  await expect(drums).toHaveCount(before);

  // And it went back where it came from: the intro still has no drums, because
  // no shipped form gives it any.
  const introDrums = page.locator(
    '[data-testid="song-section-0"] ~ .song__rows [data-testid="song-clip-drums"]',
  );
  expect(await introDrums.count()).toBeLessThanOrEqual(before);
});

test('the copy shortcuts still fire with caps lock on', async ({ page }) => {
  // `event.key` is 'C' rather than 'c' under caps lock or shift, and the
  // handler compared against lowercase literals — so every shortcut silently
  // stopped working with nothing on screen to explain why.
  await openSong(page);

  const drums = page.locator('[data-testid="song-clip-drums"]');
  const before = await drums.count();
  await drums.last().click();

  // Shift is the reachable equivalent of caps lock in a headless browser: both
  // give `event.key` the upper-case form.
  await page.keyboard.press('Control+Shift+X');
  await expect(drums).toHaveCount(before - 1);
});

// ---------------------------------------------------------------------------
// Playback controls (TASK-072).
//
// ⚠ **Nothing here can hear anything.** The mock has no audio thread, so what
// these assert is that the controls are wired to the right section and the right
// row — which is the half a screenshot cannot check either. Whether the loop
// actually turns over, and whether a muted row goes quiet, are in
// `Live-To-Do.md` § 4 because only a human with speakers can answer them.
// ---------------------------------------------------------------------------

test('a section can be looped, and looping a second one moves the loop', async ({ page }) => {
  await openSong(page);

  const first = page.locator('[data-testid="song-section-0"]');
  const second = page.locator('[data-testid="song-section-1"]');
  await expect(first).toHaveAttribute('data-looping', 'false');

  await first.getByRole('button', { name: 'Loop this section' }).click();
  await expect(first).toHaveAttribute('data-looping', 'true');

  // ⛔ One loop, not two. A per-section toggle that did not clear the others
  // would leave the transport with a loop the producer thought they had moved.
  await second.getByRole('button', { name: 'Loop this section' }).click();
  await expect(second).toHaveAttribute('data-looping', 'true');
  await expect(first).toHaveAttribute('data-looping', 'false');

  // And pressing it again plays the record through.
  await second.getByRole('button', { name: 'Loop this section' }).click();
  await expect(second).toHaveAttribute('data-looping', 'false');
});

test('a part row can be muted and soloed for the preview', async ({ page }) => {
  await openSong(page);

  const mute = page.getByRole('button', { name: /^Mute Drums/ });
  const solo = page.getByRole('button', { name: /^Solo Drums/ });

  await expect(mute).toHaveAttribute('aria-pressed', 'false');
  await mute.click();
  await expect(mute).toHaveAttribute('aria-pressed', 'true');

  await solo.click();
  await expect(solo).toHaveAttribute('aria-pressed', 'true');
  // ⚠ The mute stays set. Solo wins over it rather than clearing it, so
  // un-soloing puts the producer back where they were.
  await expect(mute).toHaveAttribute('aria-pressed', 'true');
});

test('generating a song drops a loop set on the previous one', async ({ page }) => {
  // A loop names a section by index, and a fresh song has different sections at
  // those indices — a kept loop would repeat whichever bars happened to land
  // there, which is not what the producer chose.
  await openSong(page);
  const first = page.locator('[data-testid="song-section-0"]');
  await first.getByRole('button', { name: 'Loop this section' }).click();
  await expect(first).toHaveAttribute('data-looping', 'true');

  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(first).toHaveAttribute('data-looping', 'false');
});

// ---------------------------------------------------------------------------
// Thumbnails, locks and the structure row (TASK-070).
// ---------------------------------------------------------------------------

test('every clip draws its own notes rather than a label on a box', async ({ page }) => {
  // ⛔⛔ **TASK-142's first finding: *"a clip does not look like a clip"*.** What
  // this replaced was a note-*density* gradient — sixteen buckets of "how busy
  // is this bar" — which cannot tell two clips apart and which no DAW draws.
  //
  // ⚠ **The paths are compared, not merely counted.** A renderer that emitted
  // the same `d` for every clip would pass "each clip has notes" while drawing
  // one picture five times, which is the readout-that-lies shape this whole
  // view keeps guarding against.
  await openSong(page);

  const paths = await page
    .locator('.song__note')
    .evaluateAll((nodes) => nodes.map((n) => n.getAttribute('d') ?? ''));
  expect(paths.length).toBeGreaterThan(1);
  for (const path of paths) expect(path).toMatch(/^M[\d.]+ [\d.]+h/);
  expect(new Set(paths).size).toBeGreaterThan(1);
});

test('a clip says which formats it can be handed over as', async ({ page }) => {
  // ⛔ TASK-142's second finding: *"MIDI and audio are indistinguishable — an
  // arrangement clip has no format at all."* Every clip is notes, so MIDI is
  // always offered; audio only where something in it has a sample behind it.
  await openSong(page);

  const clip = page.locator('.song__clip').first();
  await expect(clip.locator('.song__format[data-format="midi"]')).toBeVisible();
});

test('a clip can be resized to loop on fewer bars than its section', async ({ page }) => {
  // ⛔ TASK-142's third finding: *"there is no clip resize."* The section
  // handles move every row; this moves one. Driven from the keyboard because
  // the slider is a real `role="slider"` — a resize that only answered the
  // mouse would be unreachable to anyone not using one.
  await openSong(page);

  const handle = page.locator('.song__clip-resize').first();
  const before = Number(await handle.getAttribute('aria-valuenow'));
  await handle.focus();
  await page.keyboard.press('ArrowLeft');

  await expect(handle).toHaveAttribute('aria-valuenow', String(before - 1));
  // ...and the section it sits in is untouched, which is the whole distinction.
  await expect(page.getByTestId('song-section-0')).toHaveAttribute('data-bars', /\d+/);
});

test('locking a section locks every clip in it, and unlocks as one', async ({ page }) => {
  await openSong(page);

  const section = page.locator('[data-testid="song-section-1"]');
  const clipsIn = page.locator('.song__row .song__clip');
  await section.getByRole('button', { name: 'Lock this whole section' }).click();

  // Every clip standing in that section's column is now locked.
  const locked = await clipsIn.evaluateAll(
    (nodes) => nodes.filter((n) => n.getAttribute('data-locked') === 'true').length,
  );
  expect(locked).toBeGreaterThan(0);

  await section.getByRole('button', { name: 'Lock this whole section' }).click();
  const after = await clipsIn.evaluateAll(
    (nodes) => nodes.filter((n) => n.getAttribute('data-locked') === 'true').length,
  );
  expect(after).toBe(0);
});

test('locking a row locks that part in every section that plays it', async ({ page }) => {
  await openSong(page);

  const drums = page.locator('[data-testid="song-clip-drums"]');
  const total = await drums.count();
  await page.getByRole('button', { name: 'Lock Drums in every section' }).click();

  for (let i = 0; i < total; i += 1) {
    await expect(drums.nth(i)).toHaveAttribute('data-locked', 'true');
  }
});

test('the structure row names the form the song actually has', async ({ page }) => {
  await openSong(page);

  const chips = await page
    .locator('.song__structure-chip')
    .evaluateAll((nodes) => nodes.map((n) => (n.textContent ?? '').trim()));
  const kinds = (await sections(page)).map((s) => s.kind);

  // One chip per section, in playing order — a row that drifted from the
  // timeline under it would be a summary of a song that is not on screen.
  expect(chips).toHaveLength(kinds.length);
  expect(chips[0]).toBe('Intro');
});

test('the picker offers the forms the artist writes, and defaults to their choice', async ({
  page,
}) => {
  await openSong(page);

  // ⚠ A combobox since TASK-057: its rows are whole section lists joined
  // together, so they were the widest in the app and the worst served by a popup
  // the OS sized against the window.
  const pick = page.locator('.song__structure-pick');
  const picker = pick.getByRole('combobox');
  await expect(picker).toBeVisible();
  // ⛔ Absence is the default and it means "the artist chooses" — the same
  // meaning it carries for every pin in this app, and what makes two
  // generations of one artist differ. It is a named row rather than an empty
  // field, because a combobox always has something chosen.
  const anyForm = await picker.inputValue();
  expect(anyForm).not.toBe('');

  await picker.click();
  const options = page.locator('.combo__menu [role="option"]');
  expect(await options.count()).toBeGreaterThan(2);

  // The first authored form, which is the option after "any".
  const chosen = (await options.nth(1).innerText()).trim();
  await options.nth(1).click();
  await expect(picker).toHaveValue(chosen);

  // A picked form survives generating with it — it is an instruction about what
  // to build next, not a one-shot.
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(picker).toHaveValue(chosen);
});

// ---------------------------------------------------------------------------
// The three interactions TASK-071 names.
// ---------------------------------------------------------------------------

test('a clip can be auditioned on its own, and the view says so', async ({ page }) => {
  // ⛔ Audition is *visible* while it lasts. Arming one looping clip silently
  // would leave the transport playing something the timeline's own loop and
  // solo badges say it is not.
  await openSong(page);

  const clip = page.locator('[data-testid="song-clip-drums"]').first();
  await expect(page.locator('[data-testid="song-audition-stop"]')).toHaveCount(0);

  await clip.getByRole('button', { name: /^Audition the Drums clip/ }).click();
  await expect(clip).toHaveAttribute('data-auditioning', 'true');
  await expect(page.locator('[data-testid="song-audition-stop"]')).toBeVisible();

  await page.locator('[data-testid="song-audition-stop"]').click();
  await expect(clip).toHaveAttribute('data-auditioning', 'false');
});

test('R re-rolls the section the selection is in', async ({ page }) => {
  await openSong(page);

  const before = await sections(page);
  await page.locator('[data-testid="song-section-1"] .song__section-name').click();
  await page.keyboard.press('r');

  // The mock re-roll repoints the section's clips at new ids and leaves the
  // geometry alone — so what a spec can read is that the section still stands
  // and nothing else moved.
  await expect.poll(async () => (await sections(page)).length).toBe(before.length);
  expect(await sections(page)).toEqual(before);
});

test('R does not fire while a text field has focus', async ({ page }) => {
  // The same guard the copy shortcuts already have: the seed box is an <input>
  // and typing an "r" into it must not re-roll a section.
  await openSong(page);
  const seed = page.getByLabel(/^Seed/);
  await seed.fill('r');
  await expect(seed).toHaveValue('r');
});

test('double-clicking a clip opens it in its own editor', async ({ page }) => {
  await openSong(page);

  await page.locator('[data-testid="song-clip-drums"]').first().dblclick();

  // The tab moves to the part that can draw it — a melody clip left open on the
  // drum grid would be the half-done version of this gesture.
  await expect(page.getByRole('tab', { name: 'Drums' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
});

test('the song can be exported, and the chip reports what happened', async ({ page }) => {
  // ⚠ The browser mock has no native Save As and no filesystem, so it reports a
  // *cancelled* export — the one outcome that is actually true here. What this
  // asserts is the wiring and the busy state; whether a file lands where the
  // dialog said is in `Live-To-Do.md` § 4.
  await openSong(page);

  const chip = page.locator('[data-testid="song-export"]');
  await expect(chip).toBeEnabled();
  await chip.click();

  // Cancelling leaves no message — closing a Save As is the ordinary way out of
  // it, and reporting it would train people to ignore the one that matters.
  await expect.poll(async () => chip.isEnabled()).toBe(true);
  await expect(page.locator('[data-testid="song-export-note"]')).toHaveCount(0);
});

test('clicking the empty grid clears the selection rather than doing nothing', async ({
  page,
}) => {
  // ⛔ **The guard this replaces could never pass.** It was
  // `event.target !== event.currentTarget` on `.song__rows`, but every pixel of
  // that box is covered by a `.song__row` child which is a live hit-test
  // target — so `event.target` was always a row and the handler returned on its
  // first line. Click-to-seek and this background clear were both dead code,
  // and nothing covered either.
  //
  // ⚠ The *seek* half cannot be asserted here: the browser mock has no audio
  // thread, so the playhead never leaves zero. The selection clear is the
  // observable half of the same handler, and it is what proves the handler runs
  // at all. Whether the marker lands under the pointer is `Live-To-Do` § 4.5.
  await openSong(page);

  const clip = page.locator('[data-testid="song-clip-drums"]').first();
  await clip.click();
  await expect(clip).toHaveAttribute('aria-pressed', 'true');

  // ⚠ A cell that genuinely has no clip in it, found rather than guessed: the
  // fixture's intro plays *melody only*, so the first few bars of the drums row
  // are empty. The far edge of the grid is not empty — the outro plays drums —
  // and clicking a clip is the one thing this gesture must not be.
  const drumsRow = page.locator('.song__row').filter({ hasText: 'Drums' }).first();
  const box = await drumsRow.boundingBox();
  if (!box) throw new Error('the drums row has no box');
  await page.mouse.click(box.x + 30, box.y + box.height / 2);

  await expect(clip).toHaveAttribute('aria-pressed', 'false');
});

/**
 * Dragging a clip onto another section (TASK-130).
 *
 * ⛔⛔ **The one DAW verb the timeline did not have.** Mike, 2026-08-06: *"you
 * should be able to rearrange or drag them and move them, delete them, copy and
 * paste them, clone them, etc. like you would in a real DAW."* Every other verb
 * in that sentence already worked and had a test above; rearranging meant copy,
 * paste, then go back and delete the original — three gestures and three undo
 * steps for one thing a producer thinks of as a drag.
 */
test('a clip can be dragged onto another section, and leaves the first one', async ({
  page,
}) => {
  await openSong(page);

  const drums = page.locator('[data-testid="song-clip-drums"]');
  const melody = page.locator('[data-testid="song-clip-melody"]');
  const before = await drums.count();
  const layout = await sections(page);

  // The first drum clip, onto the section the *last* melody clip sits in — a
  // section that is certainly somewhere else along the timeline.
  const from = await drums.first().boundingBox();
  const to = await melody.last().boundingBox();
  if (!from || !to) throw new Error('the clips have no box');

  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  // ⚠ Stepped, because the move only becomes a drag past a 6px threshold — a
  // single jump would still cross it, but stepping is what a hand does and it
  // exercises the same path.
  await page.mouse.move(to.x + to.width / 2, to.y + from.height / 2, { steps: 8 });
  await page.mouse.up();

  // ⚠ **A move never *adds* a clip.** Not "the count is unchanged": a cell holds
  // one clip per part, so landing on a section that already has drums replaces
  // it and the count drops by one. That is the same rule paste follows, and
  // refusing the drop instead would make the gesture fail silently on what is
  // probably the commonest case.
  await expect(drums).not.toHaveCount(before + 1);
  const boxes = await drums.evaluateAll((nodes) =>
    nodes.map((n) => Math.round(n.getBoundingClientRect().x)),
  );
  expect(boxes, 'the clip did not leave the section it came from').not.toContain(
    Math.round(from.x),
  );

  // ⚠ And the sections themselves are untouched — moving a clip out of one is
  // not deleting it, the same rule `deleteClips` already holds, and the
  // arrangement must not re-tile under the producer mid-gesture.
  expect(await sections(page)).toEqual(layout);
});

test('a drag that goes nowhere is still just a click', async ({ page }) => {
  // ⛔ `click` fires after `pointerup`, so the move path has to suppress it —
  // otherwise every drag would also re-select at the far end. The inverse
  // matters just as much: a press that never travelled must still select.
  await openSong(page);

  const first = page.locator('[data-testid="song-clip-drums"]').first();
  const box = await first.boundingBox();
  if (!box) throw new Error('no box');

  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  // Two pixels — under the threshold, so this is a click with a shaky hand.
  await page.mouse.move(box.x + box.width / 2 + 2, box.y + box.height / 2);
  await page.mouse.up();

  await expect(first).toHaveAttribute('aria-pressed', 'true');
});
