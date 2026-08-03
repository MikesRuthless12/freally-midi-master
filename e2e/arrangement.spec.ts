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

test('undo on the Song tab does not silently step the session back', async ({ page }) => {
  // ⛔ **The bug this was written for, and it was watched failing.** Ctrl+Z is
  // bound globally to `useSession`'s history. The arrangement is a different
  // document with no history of its own, so on this tab the chord used to undo
  // *the session* — a seed keystroke, a pin, or a piano-roll edit made on
  // another tab — while the arrangement stayed exactly as it was. The producer
  // sees nothing happen here and finds the damage later, somewhere else.
  //
  // Doing nothing is the honest behaviour until the arrangement has its own
  // stack, so this asserts the session is untouched.
  await openSong(page);

  const seed = page.getByLabel(/^Seed/);
  const before = await seed.inputValue();

  // Something undoable in the session, then focus back on the timeline.
  await seed.fill('123456');
  await seed.press('Enter');
  await page.locator('[data-testid="song-section-0"]').click();

  const afterEdit = await seed.inputValue();
  await page.keyboard.press('Control+z');

  expect(await seed.inputValue()).toBe(afterEdit);
  expect(afterEdit).not.toBe(before);
});
