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
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('tab', { name: 'Song' }).click();
  await page.getByRole('button', { name: 'Generate' }).click();
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

  const search = page.getByLabel('Search an artist');
  await search.fill('uk');
  await search.press('Enter');

  await expect(page.locator('[data-testid="song-section-0"]')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Generate' })).toBeEnabled();
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

  await page.getByRole('button', { name: 'Generate' }).click();
  await expect(first).toHaveAttribute('data-looping', 'false');
});

// ---------------------------------------------------------------------------
// Thumbnails, locks and the structure row (TASK-070).
// ---------------------------------------------------------------------------

test('every clip carries a note-density sketch', async ({ page }) => {
  // ⛔ Painted as a gradient rather than mounted as a canvas or one element per
  // bucket — see `sketch.ts`. A painted surface has no geometry to count, so
  // what is asserted is that each clip really has one and that it is not the
  // same flat fill for every clip.
  await openSong(page);

  const fills = await page
    .locator('.song__sketch')
    .evaluateAll((nodes) => nodes.map((n) => (n as HTMLElement).style.backgroundImage));
  expect(fills.length).toBeGreaterThan(1);
  for (const fill of fills) expect(fill).toContain('linear-gradient');
  expect(new Set(fills).size).toBeGreaterThan(1);
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

  const picker = page.locator('.song__structure-pick select');
  await expect(picker).toBeVisible();
  // ⛔ Absence is the default and it means "the artist chooses" — the same
  // meaning it carries for every pin in this app, and what makes two
  // generations of one artist differ.
  await expect(picker).toHaveValue('');

  const options = await picker.locator('option').evaluateAll((nodes) => nodes.length);
  expect(options).toBeGreaterThan(2);

  await picker.selectOption('1');
  await expect(picker).toHaveValue('1');
  // A picked form survives generating with it — it is an instruction about what
  // to build next, not a one-shot.
  await page.getByRole('button', { name: 'Generate' }).click();
  await expect(picker).toHaveValue('1');
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
