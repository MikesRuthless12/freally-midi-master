import { expect, test, type Locator, type Page } from '@playwright/test';
import { pickCombo } from './app';

/**
 * The piano roll's gestures (TASK-041, TASK-041A).
 *
 * ⛔ **Every test here asserts the resulting *notes*, never that a handler
 * fired.** The roadmap asks for exactly that, and it is the only thing worth
 * asserting about a canvas: a click that lands one row off still fires the
 * handler, and a snap that rounds the wrong way still produces a note. The
 * roll publishes its notes as a visually-hidden list — the canvas's text
 * alternative — and that list is what these read.
 *
 * The view is published on the canvas as data attributes for the same reason:
 * a canvas has no geometry a test can read, so hardcoding the defaults here
 * would make every test start clicking the wrong row the day one changed.
 */

/** The view the canvas is currently drawing, in the pattern's own units. */
async function view(canvas: Locator) {
  const read = async (name: string) => Number(await canvas.getAttribute(name));
  return {
    gutter: await read('data-gutter'),
    zoom: await read('data-zoom'),
    rowHeight: await read('data-row-height'),
    scrollTick: await read('data-scroll-tick'),
    topPitch: await read('data-top-pitch'),
    ppq: await read('data-ppq'),
  };
}

/** Where a tick and a pitch sit, in page coordinates. Mirrors `geometry.ts`. */
async function pointFor(canvas: Locator, tick: number, pitch: number) {
  const v = await view(canvas);
  const box = await canvas.boundingBox();
  if (box === null) throw new Error('the roll canvas has no box');
  return {
    x: box.x + v.gutter + ((tick - v.scrollTick) / v.ppq) * v.zoom,
    y: box.y + (v.topPitch - pitch) * v.rowHeight + v.rowHeight / 2,
  };
}

/** Every note currently in the clip, read from the roll's own list. */
async function notes(page: Page) {
  return page.locator('[data-testid="roll-notes"] li').evaluateAll((items) =>
    items.map((li) => ({
      tick: Number(li.getAttribute('data-tick')),
      pitch: Number(li.getAttribute('data-pitch')),
      len: Number(li.getAttribute('data-len')),
      vel: Number(li.getAttribute('data-vel')),
      selected: li.hasAttribute('data-selected'),
    })),
  );
}

/** Generate a melody and hand back the roll's canvas. */
async function openRoll(page: Page): Promise<Locator> {
  await page.goto('/');
  const search = page.getByRole('combobox', { name: 'Roster' });
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('tab', { name: 'Melody' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  const canvas = page.locator('.roll__canvas');
  await expect(canvas).toBeVisible();
  // The framing effect runs once the notes land; waiting on a note rather than
  // on the canvas means the view has settled before anything is measured.
  await expect(page.locator('[data-testid="roll-notes"] li').first()).toBeAttached();
  return canvas;
}

test('a generated melody is drawn where it was written', async ({ page }) => {
  await openRoll(page);
  const written = await notes(page);

  expect(written.length).toBeGreaterThan(0);
  // Every note is inside MIDI's range and inside the clip — the two invariants
  // `clampNote` exists to hold.
  for (const note of written) {
    expect(note.pitch).toBeGreaterThanOrEqual(0);
    expect(note.pitch).toBeLessThanOrEqual(127);
    expect(note.len).toBeGreaterThan(0);
  }
});

test('the roll frames the register the clip was written in', async ({ page }) => {
  // ⛔ The bug this guards: a melody sits around MIDI 54–66 and the roll's
  // default top pitch is 84, so at 20 px rows the notes are several screens
  // below the fold. A producer sees a blank grid and concludes the generator
  // wrote nothing — indistinguishable from a failure.
  const canvas = await openRoll(page);
  const written = await notes(page);
  const v = await view(canvas);
  const box = await canvas.boundingBox();
  if (box === null) throw new Error('the roll canvas has no box');

  const rows = Math.floor(box.height / v.rowHeight);
  const highest = Math.max(...written.map((n) => n.pitch));
  const lowest = Math.min(...written.map((n) => n.pitch));

  expect(highest, 'the top note is below the top of the view').toBeLessThanOrEqual(v.topPitch);
  expect(lowest, 'the bottom note is above the bottom of the view').toBeGreaterThan(
    v.topPitch - rows,
  );
});

test('double-clicking empty canvas draws a note there', async ({ page }) => {
  const canvas = await openRoll(page);
  const before = await notes(page);

  // A pitch the mock's pentatonic figure does not use, so the row is certainly
  // empty and the new note is unambiguous.
  //
  // ⚠ **Found, not assumed.** This took `topPitch - 1` and stopped being empty
  // the moment `frameTo` (TASK-056 #2) started framing the clip's own register:
  // the top row is now one above the highest note, so the row below it is the
  // highest note's. Asking the notes which rows are free cannot go stale that
  // way.
  const v = await view(canvas);
  const used = new Set(before.map((note) => note.pitch));
  const rowsShown = Math.floor((await canvas.evaluate((el) => el.clientHeight)) / v.rowHeight);
  const emptyPitch = Array.from({ length: rowsShown }, (_, index) => v.topPitch - index).find(
    (pitch) => !used.has(pitch),
  );
  if (emptyPitch === undefined) throw new Error('every visible row already has a note');
  const at = await pointFor(canvas, v.ppq * 2, emptyPitch);
  await page.mouse.dblclick(at.x, at.y);

  const after = await notes(page);
  expect(after.length).toBe(before.length + 1);
  const drawn = after.find((n) => n.pitch === emptyPitch);
  expect(drawn, 'a note landed on the row that was clicked').toBeTruthy();
  expect(drawn?.selected, 'the new note is selected so it can be dragged at once').toBe(true);
});

test('dragging a note moves it, and lands one undo step', async ({ page }) => {
  const canvas = await openRoll(page);
  const before = await notes(page);
  const first = before[0];

  const from = await pointFor(canvas, first.tick + first.len / 2, first.pitch);
  const to = await pointFor(canvas, first.tick + first.len / 2 + 960, first.pitch - 2);

  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  // Several moves, so the drag genuinely streams rather than teleporting — this
  // is what would produce one undo entry per frame if the live drag were
  // committed to the pattern instead of held in `state/editing.ts`.
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2);
  await page.mouse.move(to.x, to.y);
  await page.mouse.up();

  const after = await notes(page);
  expect(after.some((n) => n.tick === first.tick + 960 && n.pitch === first.pitch - 2)).toBe(
    true,
  );
  expect(after.length, 'a move adds no notes').toBe(before.length);

  // ⛔ One Ctrl+Z puts it back. If the drag had committed per pointermove this
  // would step back through the intermediate positions instead.
  await page.keyboard.press('Control+z');
  const undone = await notes(page);
  expect(undone.some((n) => n.tick === first.tick && n.pitch === first.pitch)).toBe(true);
});

test('Delete removes the selection and Ctrl+A takes all of it', async ({ page }) => {
  await openRoll(page);
  const before = await notes(page);
  expect(before.length).toBeGreaterThan(1);

  await page.keyboard.press('Control+a');
  expect((await notes(page)).every((n) => n.selected)).toBe(true);

  await page.keyboard.press('Delete');
  expect(await notes(page)).toHaveLength(0);

  await page.keyboard.press('Control+z');
  expect(await notes(page)).toHaveLength(before.length);
});

test('the arrow keys transpose the selection and keep it selected', async ({ page }) => {
  const canvas = await openRoll(page);
  const before = await notes(page);
  const first = before[0];

  const at = await pointFor(canvas, first.tick + first.len / 2, first.pitch);
  await page.mouse.click(at.x, at.y);
  expect((await notes(page)).filter((n) => n.selected)).toHaveLength(1);

  await page.keyboard.press('ArrowUp');
  const up = await notes(page);
  const moved = up.find((n) => n.pitch === first.pitch + 1 && n.tick === first.tick);
  expect(moved, 'the note went up a semitone').toBeTruthy();
  // ⛔ A NoteId is start tick and pitch — the two things a transpose changes —
  // so the selection has to be re-derived or the next press moves nothing.
  expect(moved?.selected, 'it is still selected after moving').toBe(true);

  await page.keyboard.press('Shift+ArrowUp');
  expect((await notes(page)).some((n) => n.pitch === first.pitch + 13)).toBe(true);
});

test('Shift+D duplicates one selection-length later', async ({ page }) => {
  const canvas = await openRoll(page);
  const before = await notes(page);
  const first = before[0];

  const at = await pointFor(canvas, first.tick + first.len / 2, first.pitch);
  await page.mouse.click(at.x, at.y);
  await page.keyboard.press('Shift+D');

  const after = await notes(page);
  expect(after.length).toBe(before.length + 1);
  expect(
    after.some((n) => n.tick === first.tick + first.len && n.pitch === first.pitch),
    'the copy sits one selection-length later',
  ).toBe(true);
});

test('right-clicking a note deletes it without disturbing the rest', async ({ page }) => {
  const canvas = await openRoll(page);
  const before = await notes(page);
  const first = before[0];

  const at = await pointFor(canvas, first.tick + first.len / 2, first.pitch);
  await page.mouse.click(at.x, at.y, { button: 'right' });

  const after = await notes(page);
  expect(after.length).toBe(before.length - 1);
  expect(after.some((n) => n.tick === first.tick && n.pitch === first.pitch)).toBe(false);
});

test('folding hides rows without touching a single note', async ({ page }) => {
  // ⛔ The roadmap's own verification for TASK-041B: fold with an out-of-scale
  // note present and confirm it is still there. Folding is a *view* transform —
  // the notes do not move, a hidden row still plays, and it still exports. A
  // fold that quietly dropped a note would be indistinguishable from a delete.
  const canvas = await openRoll(page);
  const v = await view(canvas);

  // Draw a note on a row the mock's minor-pentatonic figure does not use, so it
  // is certainly out of the key the roll is tinting against.
  const outOfScale = v.topPitch - 1;
  const at = await pointFor(canvas, v.ppq * 2, outOfScale);
  await page.mouse.dblclick(at.x, at.y);
  const before = await notes(page);
  expect(before.some((n) => n.pitch === outOfScale)).toBe(true);

  await page.getByRole('button', { name: 'Fold to scale' }).click();
  await expect(page.getByRole('button', { name: 'Fold to scale' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );

  // Every note survives the fold, byte for byte.
  expect(await notes(page)).toEqual(before);

  await page.getByRole('button', { name: 'Fold', exact: true }).click();
  expect(await notes(page)).toEqual(before);
});

test('the roll and the session chip never hold two opinions about the scale', async ({
  page,
}) => {
  // ⛔ The roll's picker writes through to the session pin rather than keeping a
  // second copy. Two opinions about the key means the tinted rows say one thing
  // and the next Generate produces another.
  const canvas = await openRoll(page);
  // ⚠ A combobox since TASK-057, so it is driven by name rather than by value —
  // and 41 scales is exactly the list length that made the OS popup unusable.
  // ⛔ Scoped to the roll bar: the session chip holds a `<select>` for the very
  // same value, and a `<select>` is a `combobox` to ARIA too — which is the
  // point of this test and would otherwise make its own locator ambiguous.
  const bar = page.locator('.rollbar');
  const picker = bar.getByRole('combobox', { name: 'Scale', exact: true });

  await pickCombo(bar, 'Scale', 'Lydian');
  await expect.poll(async () => canvas.getAttribute('data-top-pitch')).not.toBeNull();
  await expect(picker).toHaveValue('Lydian');
});

test('clicking the keyboard gutter never changes the notes', async ({ page }) => {
  // The gutter auditions. A click there that edited would be the worst kind of
  // surprise — finding a register should not cost you a note.
  const canvas = await openRoll(page);
  const before = await notes(page);

  const box = await canvas.boundingBox();
  if (box === null) throw new Error('the roll canvas has no box');
  const v = await view(canvas);
  await page.mouse.click(box.x + v.gutter / 2, box.y + v.rowHeight * 3);

  expect(await notes(page)).toEqual(before);
});

/**
 * The velocity lane (TASK-041V).
 *
 * ⛔ **These assert the note's velocity, not the pixel the cap moved to.** The
 * roadmap asks for the value that reaches the export, and the roll's note list
 * is that value — it is the `vel` the SMF writer and the host emission both
 * read. A test that only checked the drawing would pass on a lane that moved
 * its own caps and wrote nothing.
 */

/** The lane's geometry, published on its canvas for the same reason the roll's is. */
async function lane(page: Page) {
  const canvas = page.locator('[data-testid="velocity-lane"]');
  const box = await canvas.boundingBox();
  if (box === null) throw new Error('the velocity lane has no box');
  return { canvas, box, gutter: Number(await canvas.getAttribute('data-gutter')) };
}

/** Where a cap sits vertically. Mirrors `velocity.ts`'s own value axis. */
function capY(vel: number, height: number): number {
  const floor = height - 2;
  const span = floor - 6;
  return floor - ((vel - 1) / 126) * span;
}

/** Every slider the lane is drawing, in the lane's own coordinates. */
async function stems(page: Page) {
  return page.locator('[data-testid="velocity-stems"] li').evaluateAll((items) =>
    items.map((li) => ({
      tick: Number(li.getAttribute('data-tick')),
      pitch: Number(li.getAttribute('data-pitch')),
      vel: Number(li.getAttribute('data-vel')),
      modelVel: Number(li.getAttribute('data-model-vel')),
      x: Number(li.getAttribute('data-x')),
    })),
  );
}

/**
 * The whole lane takes clicks — including under the controls (2026-08-06).
 *
 * ⛔⛔ **The bug this closes was live for months and nothing reported it.** The
 * error, the session prompt and the Generate row lived in a `position: absolute`
 * column pinned to the body's bottom-right corner, floating *over* the editor —
 * and the velocity lane is full width along that same bottom edge. Measured at
 * the time: the lane spanned y 748–843 and the column sat at 776–820, so
 * `document.elementFromPoint` over that region answered with a control and a cap
 * dragged under one did not move at all.
 *
 * ⚠ **It hid because the column is right-aligned.** It only covers a given cap
 * once it is wide enough to reach it, so the dead region grew silently with
 * every control added — and the test that eventually caught it
 * (`the lane and the roll never disagree about a velocity`) only did so because
 * one more button pushed the column's edge past the cap it happened to use.
 * That is luck, not coverage. This aims at the **last** cap in the lane, which
 * is the one nearest the corner the column used to occupy.
 */
test('a cap under the controls can still be dragged', async ({ page }) => {
  await openRoll(page);
  const { box, gutter } = await lane(page);
  const before = await stems(page);
  // ⚠ The rightmost cap that is actually *drawn*. The lane windows the clip, so
  // the last entry in the list can sit past the visible edge — a drag there
  // misses for an ordinary reason and would make this test look like the bug it
  // is meant to catch.
  const visible = before.filter((stem) => gutter + stem.x < box.width - 4);
  const target = visible[visible.length - 1];
  expect(target, 'no cap is drawn, so this asserts nothing').toBeDefined();

  const x = box.x + gutter + target.x;
  const y = box.y + capY(target.vel, box.height);

  // ⚠ Asserted before the drag, because a drag that lands on a control fails in
  // exactly the same way as one that lands on nothing — the note simply keeps
  // its velocity — and that is indistinguishable from the edit being refused.
  const covering = await page.evaluate(
    ([px, py]) => {
      const el = document.elementFromPoint(px as number, py as number);
      return el?.closest('.stage__bottom') === null ? 'lane' : 'controls';
    },
    [x, y],
  );
  expect(covering, 'the controls are lying over the velocity lane again').toBe('lane');

  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, box.y + capY(64, box.height), { steps: 4 });
  await page.mouse.up();

  const after = await stems(page);
  expect(after.find((stem) => stem.tick === target.tick)?.vel).toBe(64);
});

test('dragging a cap sets that note’s velocity and leaves the rest alone', async ({ page }) => {
  await openRoll(page);
  const { box, gutter } = await lane(page);
  const before = await stems(page);
  const target = before[0];

  const x = box.x + gutter + target.x;
  await page.mouse.move(x, box.y + capY(target.vel, box.height));
  await page.mouse.down();
  await page.mouse.move(x, box.y + capY(30, box.height), { steps: 4 });
  await page.mouse.up();

  const after = await stems(page);
  expect(after[0].vel).toBe(30);
  expect(after.slice(1).map((s) => s.vel)).toEqual(before.slice(1).map((s) => s.vel));

  // And it is one undo step, not one per frame of the drag.
  await page.keyboard.press('Control+z');
  expect((await stems(page))[0].vel).toBe(target.vel);
});

test('dragging flat across the lane levels every note it passes', async ({ page }) => {
  // ⛔ The gesture the lane exists for: sixteen hats to one loudness in one
  // movement. Setting them one at a time is data entry, not editing.
  await openRoll(page);
  const { box, gutter } = await lane(page);
  const before = await stems(page);
  const last = before[3];

  const y = box.y + capY(64, box.height);
  await page.mouse.move(box.x + gutter + before[0].x, y);
  await page.mouse.down();
  await page.mouse.move(box.x + gutter + last.x, y, { steps: 12 });
  await page.mouse.up();

  const after = await stems(page);
  expect(after.slice(0, 4).map((s) => s.vel)).toEqual([64, 64, 64, 64]);
  // Everything past where the pointer stopped keeps what it had.
  expect(after[4].vel).toBe(before[4].vel);
});

test('right-clicking a cap puts back the velocity the model wrote', async ({ page }) => {
  // ⛔ The default is the *model's* value, not a flat 100 — resetting to a
  // constant would quietly delete the accent pattern that makes a generated
  // part sound played rather than programmed.
  await openRoll(page);
  const { box, gutter } = await lane(page);
  const before = await stems(page);
  const target = before[0];
  expect(target.modelVel, 'the fixture must carry a model velocity to restore').toBeGreaterThan(
    0,
  );

  const x = box.x + gutter + target.x;
  await page.mouse.move(x, box.y + capY(target.vel, box.height));
  await page.mouse.down();
  await page.mouse.move(x, box.y + capY(12, box.height), { steps: 4 });
  await page.mouse.up();
  expect((await stems(page))[0].vel).toBe(12);

  await page.mouse.click(x, box.y + capY(12, box.height), { button: 'right' });
  expect((await stems(page))[0].vel).toBe(target.modelVel);
});

test('the lane and the roll never disagree about a velocity', async ({ page }) => {
  // Two views of one number. The roll's list is what exports; the lane's is what
  // the caps are drawn from, and a lane editing its own copy would be invisible
  // until the producer opened the file somewhere else.
  await openRoll(page);
  const { box, gutter } = await lane(page);
  const target = (await stems(page))[2];

  const x = box.x + gutter + target.x;
  await page.mouse.move(x, box.y + capY(target.vel, box.height));
  await page.mouse.down();
  await page.mouse.move(x, box.y + capY(96, box.height), { steps: 4 });
  await page.mouse.up();

  const written = (await notes(page)).find((n) => n.tick === target.tick);
  expect(written?.vel).toBe(96);
});

/**
 * The transform menu and its two shortcuts (TASK-041D).
 *
 * The transforms themselves are asserted note by note in
 * `transforms.test.ts` — a Playwright pass proves the menu is wired to them,
 * that the result is one undo step, and that the selection survives so a
 * producer can chain two.
 */

/** Select every note in the clip, which is what the menu items act on. */
async function selectAll(page: Page) {
  await page.keyboard.press('Control+a');
  await expect(
    page.locator('[data-testid="roll-notes"] li[data-selected]').first(),
  ).toBeAttached();
}

test('the transform menu reverses the selection, in one undo step', async ({ page }) => {
  await openRoll(page);
  const before = await notes(page);
  await selectAll(page);

  await page.getByRole('button', { name: 'Transform' }).click();
  await page.getByRole('menuitem', { name: 'Reverse' }).click();

  const after = await notes(page);
  // A mirror in time: the pitch that was last is now first, at the same span.
  expect(after[0].pitch).toBe(before[before.length - 1].pitch);
  expect(after.length).toBe(before.length);

  await page.keyboard.press('Control+z');
  expect(await notes(page)).toEqual(before);
});

test('a transform keeps its own result selected, so two can be chained', async ({ page }) => {
  // ⛔ A `NoteId` is a start tick and a pitch, and every transform moves one.
  // Without re-deriving the selection the second press would act on notes that
  // no longer exist and look like the menu had stopped working.
  await openRoll(page);
  await selectAll(page);
  const count = (await notes(page)).length;

  await page.getByRole('button', { name: 'Transform' }).click();
  await page.getByRole('menuitem', { name: 'Invert up' }).click();
  expect((await notes(page)).filter((n) => n.selected).length).toBe(count);

  await page.getByRole('button', { name: 'Transform' }).click();
  await page.getByRole('menuitem', { name: 'Legato' }).click();
  // Every note now reaches the next onset, so nothing keeps the original gap.
  const after = await notes(page);
  expect(after[0].len).toBe(after[1].tick - after[0].tick);
});

test('the menu says a selection is what it wants, rather than doing nothing', async ({
  page,
}) => {
  await openRoll(page);
  await page.keyboard.press('Escape');

  await page.getByRole('button', { name: 'Transform' }).click();
  await expect(page.getByRole('menuitem', { name: 'Reverse' })).toBeDisabled();
});

test('* and / stretch and compress the selection', async ({ page }) => {
  await openRoll(page);
  await selectAll(page);
  const before = await notes(page);

  await page.keyboard.press('/');
  const halved = await notes(page);
  expect(halved[1].tick - halved[0].tick).toBe((before[1].tick - before[0].tick) / 2);

  await page.keyboard.press('*');
  const back = await notes(page);
  expect(back[1].tick - back[0].tick).toBe(before[1].tick - before[0].tick);
});

/**
 * The ruler, the loop brace, the clip markers and the meter (TASK-041E).
 *
 * The strip publishes the two regions as data attributes for the same reason
 * the roll publishes its notes: a canvas has no geometry a test can read, and
 * mirroring the layout here would drift from it the first time it changed.
 */

/** The ruler's geometry, and where a tick sits along it. */
async function ruler(page: Page) {
  const canvas = page.locator('[data-testid="roll-ruler"]');
  const box = await canvas.boundingBox();
  if (box === null) throw new Error('the ruler has no box');
  const roll = page.locator('.roll__canvas');
  const v = await view(roll);
  return {
    canvas,
    box,
    at: (tick: number) => box.x + v.gutter + ((tick - v.scrollTick) / v.ppq) * v.zoom,
    y: box.y + 4,
    bar: v.ppq * 4,
    read: async (name: string) => Number(await canvas.getAttribute(name)),
  };
}

test('dragging the brace sets a loop region the clip carries', async ({ page }) => {
  await openRoll(page);
  const r = await ruler(page);

  // The whole clip until someone drags: the brace starts where the clip does.
  expect(await r.read('data-loop-from')).toBe(0);

  // ⛔ Started away from either end of the default brace. A drag that begins on
  // a handle *moves* that handle — which is the other gesture, and the one the
  // next test covers.
  await page.mouse.move(r.at(r.bar), r.y);
  await page.mouse.down();
  await page.mouse.move(r.at(r.bar * 3), r.y, { steps: 8 });
  await page.mouse.up();

  expect(await r.read('data-loop-from')).toBe(r.bar);
  expect(await r.read('data-loop-to')).toBe(r.bar * 3);

  // It is one clip edit, so it steps back like any other.
  await page.keyboard.press('Control+z');
  expect(await r.read('data-loop-from')).toBe(0);
});

test('the loop’s end can be dragged without moving its start', async ({ page }) => {
  await openRoll(page);
  const r = await ruler(page);

  await page.mouse.move(r.at(r.bar), r.y);
  await page.mouse.down();
  await page.mouse.move(r.at(r.bar * 2), r.y, { steps: 6 });
  await page.mouse.up();

  // Now grab the end handle and pull it out by a bar.
  await page.mouse.move(r.at(r.bar * 2), r.y);
  await page.mouse.down();
  await page.mouse.move(r.at(r.bar * 3), r.y, { steps: 6 });
  await page.mouse.up();

  expect(await r.read('data-loop-from')).toBe(r.bar);
  expect(await r.read('data-loop-to')).toBe(r.bar * 3);
});

test('the clip markers are a separate pair from the loop', async ({ page }) => {
  // ⛔ Two regions, not one. Looping bar 2 to work on it must not trim the clip
  // to bar 2, which is the whole reason they are separate fields.
  await openRoll(page);
  const r = await ruler(page);
  const clipEnd = await r.read('data-clip-to');

  await page.mouse.move(r.at(r.bar), r.y);
  await page.mouse.down();
  await page.mouse.move(r.at(r.bar * 2), r.y, { steps: 6 });
  await page.mouse.up();

  expect(await r.read('data-loop-to')).toBe(r.bar * 2);
  expect(await r.read('data-clip-to')).toBe(clipEnd);
});

test('the meter picker changes the clip and the bars drawn under it', async ({ page }) => {
  // ⛔ It writes the *pattern*, because the pattern is what exports — a clip
  // that said 6/8 on screen and 4/4 in the file would be the readout-that-lies
  // failure in its most expensive form: found in another DAW, days later.
  await openRoll(page);
  const bar = page.locator('.rollbar');
  const meter = bar.getByRole('combobox', { name: 'Meter', exact: true });
  await expect(meter).toHaveValue('4/4');

  await pickCombo(bar, 'Meter', '6/8');
  await expect(meter).toHaveValue('6/8');

  // A 6/8 bar is three quarter notes, so the clip's own bar length halves.
  const r = await ruler(page);
  const roll = page.locator('.roll__canvas');
  const v = await view(roll);
  expect(await r.read('data-clip-to')).toBe(v.ppq * 3 * 4);
});

/**
 * The cursors, and what a plain click does (Mike, 2026-08-06).
 *
 * *"i do not want the '+' cursor for the piano rolls, only the '[' and ']'
 * cursors for resizing notes and a regular mousepointer, also, just clicking on
 * any single note should select that note"* … *"can you also ensure that we
 * have this for extending/shortening a note for the left and right sides of all
 * generators."*
 */
test('the roll wears a plain pointer over empty grid, never a crosshair', async ({ page }) => {
  // ⛔ A "+" is the cursor a *drawing* tool wears. It promised an interaction
  // this roll does not offer — clicking empty space selects nothing, it does
  // not add a note — so it was answering the producer's question wrongly.
  const canvas = await openRoll(page);
  const box = await canvas.boundingBox();
  if (!box) throw new Error('no canvas');

  // Well below the notes, where nothing is drawn.
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height - 8);
  await expect(canvas).toHaveCSS('cursor', 'default');
});

test('each edge of a note wears the cursor for the end it would move', async ({ page }) => {
  // ⛔ `w-resize` and `e-resize`, not one `ew-resize` for both. Which way the
  // bracket faces says which end is about to move: the left edge changes where
  // the note starts, the right edge changes how long it is.
  const canvas = await openRoll(page);
  const first = (await notes(page))[0];

  const left = await pointFor(canvas, first.tick, first.pitch);
  await page.mouse.move(left.x + 2, left.y);
  await expect(canvas).toHaveCSS('cursor', 'w-resize');

  const right = await pointFor(canvas, first.tick + first.len, first.pitch);
  await page.mouse.move(right.x - 2, right.y);
  await expect(canvas).toHaveCSS('cursor', 'e-resize');

  // The body between them moves the note rather than resizing it.
  const middle = await pointFor(canvas, first.tick + first.len / 2, first.pitch);
  await page.mouse.move(middle.x, middle.y);
  await expect(canvas).toHaveCSS('cursor', 'move');
});

test('one plain click selects a note, with no modifier held', async ({ page }) => {
  // ⚠ Mike reported having to Ctrl+click. The code already did this, so what
  // was missing was a gate saying so — and probably the crosshair above, which
  // made the roll read as a tool that draws rather than one that selects.
  const canvas = await openRoll(page);
  const first = (await notes(page))[0];

  const at = await pointFor(canvas, first.tick + first.len / 2, first.pitch);
  await page.mouse.click(at.x, at.y);

  const selected = (await notes(page)).filter((n) => n.selected);
  expect(selected).toHaveLength(1);
  expect(selected[0].pitch).toBe(first.pitch);
});

/**
 * Transposing the selection (Mike, 2026-08-06).
 *
 * *"if you have a single note or multiple notes selected, then 'Ctrl+up arrow'
 * should move the note/notes up a single half step, and if you press 'Shift+up
 * arrow' then it should move them up a whole octave, the same goes with
 * 'Ctrl+down arrow' or 'Shift+down arrow'."*
 *
 * ⚠ The bindings already behaved this way — nothing guards the arrows on Ctrl,
 * so it falls through to the semitone step — but nothing proved it either, and
 * a binding that works by not being excluded is one a later guard could take
 * away silently.
 */
test('Ctrl and Shift with the arrows move by a semitone and an octave', async ({ page }) => {
  const canvas = await openRoll(page);
  const first = (await notes(page))[0];
  const at = await pointFor(canvas, first.tick + first.len / 2, first.pitch);
  await page.mouse.click(at.x, at.y);

  await page.keyboard.press('Control+ArrowUp');
  expect(
    (await notes(page)).some((n) => n.pitch === first.pitch + 1 && n.tick === first.tick),
    'Ctrl+Up should be one half step',
  ).toBe(true);

  await page.keyboard.press('Control+ArrowDown');
  expect((await notes(page)).some((n) => n.pitch === first.pitch)).toBe(true);

  await page.keyboard.press('Shift+ArrowUp');
  expect(
    (await notes(page)).some((n) => n.pitch === first.pitch + 12),
    'Shift+Up should be a whole octave',
  ).toBe(true);

  await page.keyboard.press('Shift+ArrowDown');
  expect((await notes(page)).some((n) => n.pitch === first.pitch)).toBe(true);
});

test('the arrows move every selected note, not just one', async ({ page }) => {
  // "a single note or multiple notes selected" — his words, and the case that
  // would break if the handler ever read a single anchor instead of the set.
  const canvas = await openRoll(page);
  await canvas.click();
  await page.keyboard.press('Control+a');

  const before = await notes(page);
  expect(before.length, 'the fixture needs more than one note').toBeGreaterThan(1);

  await page.keyboard.press('Control+ArrowUp');
  const after = await notes(page);

  for (const note of before) {
    expect(
      after.some((n) => n.tick === note.tick && n.pitch === note.pitch + 1),
      `the note at tick ${note.tick} did not move`,
    ).toBe(true);
  }
});
