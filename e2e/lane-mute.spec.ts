import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

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

  await pickArtist(page, 'Mock Artist');
  await page.getByRole('button', { name: 'Generate', exact: true }).first().click();
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

test('the kit’s velocity lane edits the drum note it sits under', async ({ page }) => {
  // TASK-041V's second half: the lane is under the drum grid as well as the
  // roll, and it is the only thing on that screen that edits — a velocity is
  // not a note, so the grid itself stays read-only until the pads are editable.
  //
  // ⛔ The stem's own lane is asserted, not just the value. The kit's lane draws
  // every drum at once, so a value written back by tick and pitch alone could
  // land on the 808 while the producer was dragging the kick.
  const lane = page.locator('[data-testid="velocity-lane"]');
  await expect(lane).toBeVisible();

  const box = await lane.boundingBox();
  if (box === null) throw new Error('the velocity lane has no box');

  const stems = () =>
    page.locator('[data-testid="velocity-stems"] li').evaluateAll((items) =>
      items.map((li) => ({
        lane: li.getAttribute('data-lane'),
        vel: Number(li.getAttribute('data-vel')),
        x: Number(li.getAttribute('data-x')),
      })),
    );

  const before = await stems();
  expect(before.length).toBeGreaterThan(0);
  const target = before[0];

  // The value axis, mirrored from `velocity.ts` exactly as the roll's spec does.
  const capY = (vel: number) => box.y + (box.height - 2) - ((vel - 1) / 126) * (box.height - 8);

  await page.mouse.move(box.x + target.x, capY(target.vel));
  await page.mouse.down();
  await page.mouse.move(box.x + target.x, capY(50), { steps: 4 });
  await page.mouse.up();

  const after = await stems();
  expect(after[0]).toMatchObject({ lane: target.lane, vel: 50 });
  expect(after.slice(1).map((s) => s.vel)).toEqual(before.slice(1).map((s) => s.vel));
});

/**
 * Solo, the audition and the roll palette (TASK-043).
 *
 * `plugin/src/shared.rs::set_lane_audio` owns what solo *means* to the audio
 * thread — that it silences every lane it does not name, that an empty set is
 * "no solo" rather than "solo nothing", and that a mute outranks it. What only
 * a browser shows is that the control is on the row it claims to be on, and
 * that the row's dimming follows what a producer can actually hear rather than
 * what its own buttons say.
 */

test('soloing a lane dims every other row without pressing their mutes', async ({ page }) => {
  const rows = page.locator('.grid__row');
  const kick = rows.first();
  const snare = rows.nth(1);

  await kick.locator('.grid__solo').click();

  await expect(kick.locator('.grid__solo')).toHaveAttribute('aria-pressed', 'true');
  await expect(kick).not.toHaveAttribute('data-muted', 'true');

  // ⛔ The row next door is silent — but its *mute* is still off. Reading the
  // dimming off `mutedLanes` alone would have left every un-soloed row looking
  // live while playing nothing, which is the readout-that-lies failure in the
  // one control that says what you can hear.
  await expect(snare).toHaveAttribute('data-muted', 'true');
  await expect(snare.locator('.grid__mute')).toHaveAttribute('aria-pressed', 'false');

  // And un-soloing puts everything back, rather than leaving the rows it dimmed
  // muted for good.
  await kick.locator('.grid__solo').click();
  await expect(snare).not.toHaveAttribute('data-muted', 'true');
});

test('a muted lane stays muted through a solo that names it', async ({ page }) => {
  // Mute wins, both here and on the audio thread. A solo that could un-mute
  // would leave the lane audible once the solo came off, with the mute lit.
  const kick = page.locator('.grid__row').first();

  await kick.locator('.grid__mute').click();
  await kick.locator('.grid__solo').click();

  await expect(kick.locator('.grid__mute')).toHaveAttribute('aria-pressed', 'true');
  await expect(kick.locator('.grid__solo')).toHaveAttribute('aria-pressed', 'true');
  await expect(kick).toHaveAttribute('data-muted', 'true');
});

test('the lane’s name is the audition button, and says so', async ({ page }) => {
  // Mike's ask was that clicking a lane's *header* plays that pad on its own,
  // so the target is the name — the largest thing in the header — rather than a
  // fourth icon nobody would find. ⚠ The mock has no audio thread, so what is
  // asserted is that the control exists, is reachable and is labelled; the
  // sound is `plugin/src/editor.rs`'s.
  const name = page.locator('.grid__row').first().locator('.grid__lanename');
  await expect(name).toHaveRole('button');
  await expect(name).toHaveAttribute('aria-label', /Hear /);
  await name.click();
  // Clicking it is not an edit: the pattern is untouched.
  await expect(page.locator('.grid__row').first()).not.toHaveAttribute('data-muted', 'true');
});

test('right-clicking a cell offers the roll palette, and picking one fills the step', async ({
  page,
}) => {
  const cell = page.locator('.grid__row').first().locator('.grid__cell').first();
  await cell.click({ button: 'right' });

  const palette = page.getByRole('menu', { name: 'Roll this step' });
  await expect(palette).toBeVisible();

  // ⛔ The count is what the palette promises, and `data-hits` is what the cell
  // reports — so this is the gesture end to end rather than a menu that opens.
  await palette.getByRole('menuitem', { name: '4 hits' }).click();
  await expect(palette).toBeHidden();
  await expect(cell).toHaveAttribute('data-hits', '4');

  // One press of Ctrl+Z takes the whole roll back, not four.
  await page.keyboard.press('Control+z');
  await expect(cell).not.toHaveAttribute('data-hits', '4');
});

test('right-dragging across cells clears them all, in one undo step', async ({ page }) => {
  // TASK-056 #3. Mike: "right-click and drag across the drum grid to delete —
  // multiple cells in one gesture, not one click each."
  //
  // ⛔ Only a browser can prove this one. The split between "a right-click opens
  // the palette" and "a right-drag deletes" is decided by whether the pointer
  // moved between down and up, and that ordering — pointerup, then contextmenu —
  // is the browser's, not something a unit test can assert.
  const row = page.locator('.grid__row').first();
  const cells = row.locator('.grid__cell');
  const lit = row.locator('.grid__cell--on');

  const before = await lit.count();
  expect(before).toBeGreaterThan(1);

  const first = await cells.nth(0).boundingBox();
  const last = await cells.nth((await cells.count()) - 1).boundingBox();
  if (first === null || last === null) throw new Error('the grid has no cells to sweep');

  await page.mouse.move(first.x + first.width / 2, first.y + first.height / 2);
  await page.mouse.down({ button: 'right' });
  // ⚠ Enough steps to land inside every cell on the way. A coarse sweep skips
  // cells, which is true of a real drag too — but then the test would be
  // asserting the interpolation rather than the gesture.
  await page.mouse.move(last.x + last.width / 2, last.y + last.height / 2, { steps: 400 });
  await page.mouse.up({ button: 'right' });

  // The whole row is wiped, and the palette the same button opens did not.
  await expect(lit).toHaveCount(0);
  await expect(page.getByRole('menu', { name: 'Roll this step' })).toBeHidden();

  // ⛔ One press, not one per cell — the reason `clearCells` exists.
  await page.keyboard.press('Control+z');
  await expect(lit).toHaveCount(before);
});

test('a right-click that does not travel still opens the palette', async ({ page }) => {
  // The other half of the split: the sweep must not have cost TASK-043 its
  // gesture. Down and up on one cell with no movement is a menu.
  const cell = page.locator('.grid__row').first().locator('.grid__cell').first();
  const box = await cell.boundingBox();
  if (box === null) throw new Error('the grid has no cells');

  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down({ button: 'right' });
  await page.mouse.up({ button: 'right' });

  await expect(page.getByRole('menu', { name: 'Roll this step' })).toBeVisible();
});

/**
 * Selection, copy, paste and clone in the drum grid (TASK-056 #4).
 *
 * Mike: *"select, copy, paste and clone multiple notes in the drum grid — i
 * don't care if you want me to press 'Ctrl+click' or whatever."* The arithmetic
 * lives in `cells.test.ts`; what only a browser proves is that the modifiers
 * reach the handlers and that selecting a cell does not also edit it.
 */
test('Ctrl-clicking selects cells without placing or clearing a hit', async ({ page }) => {
  const cells = page.locator('.grid__row').first().locator('.grid__cell');
  const before = await cells.evaluateAll((els) =>
    els.map((el) => el.getAttribute('data-hits')),
  );

  await cells.nth(2).click({ modifiers: ['Control'] });
  await cells.nth(5).click({ modifiers: ['Control'] });

  await expect(cells.nth(2)).toHaveAttribute('data-selected', 'true');
  await expect(cells.nth(5)).toHaveAttribute('data-selected', 'true');
  await expect(cells.nth(3)).not.toHaveAttribute('data-selected', 'true');

  // ⛔ The pattern is untouched. A selection that also toggled a hit would make
  // every act of selecting an edit.
  expect(
    await cells.evaluateAll((els) => els.map((el) => el.getAttribute('data-hits'))),
  ).toEqual(before);

  // And a plain click starts again, the way it does in every editor.
  await cells.nth(9).click();
  await expect(cells.nth(2)).not.toHaveAttribute('data-selected', 'true');
});

test('shift-dragging a bar and pasting it lands the same figure in the next one', async ({
  page,
}) => {
  const row = page.locator('.grid__row').first();
  const cells = row.locator('.grid__cell');
  const total = await cells.count();
  expect(total).toBeGreaterThanOrEqual(32);

  const hits = () => cells.evaluateAll((els) => els.map((el) => el.getAttribute('data-hits')));
  const first = await cells.nth(0).boundingBox();
  const sixteenth = await cells.nth(15).boundingBox();
  if (first === null || sixteenth === null) throw new Error('the grid has no cells to sweep');

  // The marquee: Shift with the primary button, so click-to-seek keeps the
  // plain drag it has always had.
  await page.keyboard.down('Shift');
  await page.mouse.move(first.x + first.width / 2, first.y + first.height / 2);
  await page.mouse.down();
  await page.mouse.move(sixteenth.x + sixteenth.width / 2, sixteenth.y + sixteenth.height / 2, {
    steps: 120,
  });
  await page.mouse.up();
  await page.keyboard.up('Shift');

  await expect(cells.nth(0)).toHaveAttribute('data-selected', 'true');
  await expect(cells.nth(15)).toHaveAttribute('data-selected', 'true');
  await expect(cells.nth(20)).not.toHaveAttribute('data-selected', 'true');

  const barOne = (await hits()).slice(0, 16);
  await page.keyboard.press('Control+c');

  // ⛔ Focused, not clicked: a plain click would clear the selection and toggle
  // a hit. The focused cell's column is where the block lands.
  await cells.nth(16).focus();
  await page.keyboard.press('Control+v');

  await expect(async () => {
    expect((await hits()).slice(16, 32)).toEqual(barOne);
  }).toPass();
});

test('dragging draws a box, and it selects the rectangle rather than the path', async ({
  page,
}) => {
  // Mike, 2026-08-11: "creates a box as you drag like on windows and then you
  // select multiple drum notes with drag."
  //
  // ⛔⛔ **This is the test that would have caught the first cut.** That version
  // collected cells from `pointerenter`, so it selected the diagonal line the
  // pointer travelled and left everything above and below it alone. The
  // assertion below is deliberately about a cell the pointer NEVER TOUCHES: the
  // drag goes from row 1 column 1 to row 3 column 5, and row 2 column 3 sits
  // inside the rectangle but nowhere near the diagonal.
  const rows = page.locator('.grid__row');
  expect(await rows.count()).toBeGreaterThanOrEqual(3);

  const start = await rows.nth(0).locator('.grid__cell').nth(1).boundingBox();
  const end = await rows.nth(2).locator('.grid__cell').nth(5).boundingBox();
  if (start === null || end === null) throw new Error('the grid has no cells to band');

  await page.keyboard.down('Shift');
  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
  await page.mouse.down();
  await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 20 });

  // The band is drawn while the drag is live, which is the visible half of it.
  await expect(page.getByTestId('grid-marquee')).toBeVisible();

  await page.mouse.up();
  await page.keyboard.up('Shift');

  // ...and gone once it is released.
  await expect(page.getByTestId('grid-marquee')).toBeHidden();

  // Everything inside the rectangle, including the middle row the pointer never
  // crossed.
  await expect(rows.nth(1).locator('.grid__cell').nth(3)).toHaveAttribute(
    'data-selected',
    'true',
  );
  await expect(rows.nth(0).locator('.grid__cell').nth(2)).toHaveAttribute(
    'data-selected',
    'true',
  );
  await expect(rows.nth(2).locator('.grid__cell').nth(4)).toHaveAttribute(
    'data-selected',
    'true',
  );

  // ...and nothing outside it.
  await expect(rows.nth(0).locator('.grid__cell').nth(9)).not.toHaveAttribute(
    'data-selected',
    'true',
  );
});

test('Delete clears a whole selection, and one Ctrl+Z brings it all back', async ({ page }) => {
  const row = page.locator('.grid__row').first();
  const cells = row.locator('.grid__cell');
  const lit = row.locator('.grid__cell--on');
  const before = await lit.count();
  expect(before).toBeGreaterThan(1);

  await cells.nth(0).click({ modifiers: ['Control'] });
  await cells.nth(1).click({ modifiers: ['Control'] });
  await cells.nth(2).click({ modifiers: ['Control'] });
  await cells.nth(3).click({ modifiers: ['Control'] });

  const litInBlock = await cells.evaluateAll(
    (els) => els.slice(0, 4).filter((el) => el.classList.contains('grid__cell--on')).length,
  );
  expect(litInBlock).toBeGreaterThan(0);

  await page.keyboard.press('Delete');
  await expect(lit).toHaveCount(before - litInBlock);

  // ⛔ One press for the whole selection — the reason `clearCells` builds one
  // pattern rather than folding `clearCell` over the list.
  await page.keyboard.press('Control+z');
  await expect(lit).toHaveCount(before);
});

test('Escape dismisses the roll palette without changing the step', async ({ page }) => {
  const cell = page.locator('.grid__row').first().locator('.grid__cell').first();
  const before = await cell.getAttribute('data-hits');

  await cell.click({ button: 'right' });
  await expect(page.getByRole('menu', { name: 'Roll this step' })).toBeVisible();
  await page.keyboard.press('Escape');

  await expect(page.getByRole('menu', { name: 'Roll this step' })).toBeHidden();
  expect(await cell.getAttribute('data-hits')).toBe(before);
});

test('the hat lane offers an add-fill, and it writes one at the phrase end', async ({
  page,
}) => {
  // TASK-043H's UI half. ⛔ **The hats only** — `hihat.fill` is authored on the
  // hi-hat block alone, so a button on the kick would offer a gesture the
  // generator has no counterpart for, and pressing Generate would erase it.
  const rows = page.locator('.grid__row');
  const hat = rows.filter({ has: page.locator('.grid__fill') }).first();
  await expect(hat).toBeVisible();
  await expect(hat.locator('.grid__lanename')).toHaveText(/hat/i);

  const cells = hat.locator('.grid__cell');
  const total = await cells.count();
  const last = cells.nth(total - 1);

  // ⚠ Hover the row first. The fill button collapses to zero width until the row
  // is reached for — that is what gives the lane its name back in a 5.5rem
  // header — so it has no box to click until then. A producer hovers on the way
  // to it; Playwright has to be told.
  await hat.hover();
  await hat.locator('.grid__fill').click();

  // A 32nd stream through the last beat: every one of the last four cells now
  // holds two hits.
  for (let step = total - 4; step < total; step += 1) {
    await expect(cells.nth(step)).toHaveAttribute('data-hits', '2');
  }
  // ⚠ And it is one edit, so one Ctrl+Z takes the whole figure back rather than
  // eight presses for eight notes.
  await page.keyboard.press('Control+z');
  await expect(last).not.toHaveAttribute('data-hits', '2');
});

test('a slot can be switched to a drum the kit is not already using', async ({ page }) => {
  // TASK-043A. ⛔ **The picker offers the unused lanes only** — two slots
  // claiming one lane is a row the producer edits and cannot hear.
  const rows = page.locator('.grid__row');
  const before = await rows.count();
  const first = rows.first();
  // ⚠ A combobox since TASK-057, so the offered lanes live in a portalled
  // listbox that exists only while it is open — a `<select>`'s `<option>`s were
  // always in the DOM. The picker is still an arrow and nothing else.
  const picker = first.locator('.grid__slot').getByRole('combobox');
  await expect(picker).toHaveAttribute('aria-label', /Change /);

  // Every lane already drawn is absent from the list, bar this row's own.
  const drawn = await rows.locator('.grid__lanename').allInnerTexts();
  await picker.click();
  const offered = await page.locator('.combo__menu [role="option"]').allInnerTexts();
  const own = offered[0];
  for (const name of drawn.filter((n) => n !== own)) {
    expect(offered).not.toContain(name);
  }
  expect(offered).toContain('Conga');

  await page.locator('.combo__menu [role="option"]', { hasText: 'Conga' }).first().click();

  // The row is a conga now — the same count of rows, one of them renamed.
  await expect(rows).toHaveCount(before);
  await expect(page.locator('.grid__lanename', { hasText: 'Conga' })).toHaveCount(1);
  await expect(page.locator('.grid__lanename', { hasText: own })).toHaveCount(0);

  // ⚠ And it is one edit, so it steps back like any other.
  await page.keyboard.press('Control+z');
  await expect(page.locator('.grid__lanename', { hasText: own })).toHaveCount(1);
});

test('a locked lane survives a re-roll, and R is what re-rolls', async ({ page }) => {
  // TASK-044. ⛔ **The lock is applied to the engine's *answer*, not sent to
  // it** — generation is a pure function of its seeds, so the only exact way to
  // keep the take on screen is to keep it.
  const kick = page.locator('.grid__row').first();
  const cells = () =>
    kick
      .locator('.grid__cell')
      .evaluateAll((els) => els.map((e) => e.getAttribute('data-hits')));

  // ⚠ Hover first: the padlock collapses to zero width until the row is reached
  // for, which is what frees the 5.5rem header for the lane's name.
  await kick.hover();
  await kick.locator('.grid__lock').click();
  await expect(kick.locator('.grid__lock')).toHaveAttribute('aria-pressed', 'true');
  await expect(kick).toHaveAttribute('data-locked', 'true');

  // ⛔ And once it IS locked the padlock stays put with the pointer away — a
  // locked lane that looked unlocked until you hovered it would be the
  // readout-that-lies failure in the one control whose job is to report state.
  await page.locator('.grid__meta').hover();
  await expect(kick.locator('.grid__lock')).toBeVisible();

  const held = await cells();

  // R rerolls — and it is Generate, so the whole clip is redrawn around the
  // lane that is held.
  await page.keyboard.press('r');
  await expect.poll(cells).toEqual(held);

  // Unlocking lets it go: press R again and it may move. ⚠ Asserted as "the
  // lock is off" rather than "the notes changed", because a seed is entitled to
  // land on the same kick twice and a flake here would be worse than the gap.
  await kick.hover();
  await kick.locator('.grid__lock').click();
  await expect(kick).not.toHaveAttribute('data-locked', 'true');
});

test('L locks the row that has focus', async ({ page }) => {
  // ⛔ **On the row, not on the window.** The grid has no selection model and
  // seventeen rows, so a global L would have to guess which lane was meant.
  const snare = page.locator('.grid__row').nth(1);
  await snare.locator('.grid__mute').focus();
  await page.keyboard.press('l');

  await expect(snare.locator('.grid__lock')).toHaveAttribute('aria-pressed', 'true');
  // And only that row.
  await expect(page.locator('.grid__row').first().locator('.grid__lock')).toHaveAttribute(
    'aria-pressed',
    'false',
  );

  await page.keyboard.press('l');
  await expect(snare.locator('.grid__lock')).toHaveAttribute('aria-pressed', 'false');
});
