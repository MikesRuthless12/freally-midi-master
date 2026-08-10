import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

/**
 * The transport playhead (TASK-041T).
 *
 * `plugin/src/voice.rs`'s `transport_tests` own the semantics — that a seek
 * rewinds the cursor, that Stop returns to zero and keeps the pattern armed,
 * that Pause is the absence of advancing. What only a browser shows is that a
 * click on the grid becomes a position, and that the marker is drawn where the
 * click landed rather than a lane-gutter's width away from it.
 *
 * ⛔ The mock has no audio thread, so nothing here asserts the marker *moves* on
 * its own — that is the plugin's job and the Rust tests are what hold it. What
 * this can prove is the wiring, and the wiring is what was missing.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  // A pattern has to exist before there is a grid to click.
  await pickArtist(page, 'Mock Artist');
  await page.getByRole('button', { name: 'Generate', exact: true }).first().click();
  await expect(page.locator('.grid__track').first()).toBeVisible();
});

test('clicking the grid moves the playhead to where it was clicked', async ({ page }) => {
  const track = page.locator('.grid__track').first();
  const box = await track.boundingBox();
  if (!box) throw new Error('the track should be laid out');

  // Three quarters along.
  await page.mouse.click(box.x + box.width * 0.75, box.y + box.height / 2);

  const marker = page.locator('.grid__playhead');
  await expect(marker).toBeVisible();

  // ⛔ **The marker's rendered position, not the `--playhead` variable.**
  // Reading the custom property back asserts that the click handler stored what
  // the click handler computed — it is true whatever the CSS does, so deleting
  // the lane-gutter terms from `inset-inline-start` left it green while every
  // user saw the line ~5.5rem left of the beat it marks. Comparing the drawn
  // box against the clicked track is what actually pins the geometry.
  const line = await marker.boundingBox();
  if (!line) throw new Error('the playhead should be laid out');
  const expected = box.x + box.width * 0.75;
  expect(Math.abs(line.x - expected)).toBeLessThan(6);
});

test('a click at the far left seeks to the start rather than doing nothing', async ({
  page,
}) => {
  const track = page.locator('.grid__track').first();
  const box = await track.boundingBox();
  if (!box) throw new Error('the track should be laid out');

  // Move away from zero first, so seeking back to it is a real change.
  await page.mouse.click(box.x + box.width * 0.6, box.y + box.height / 2);
  await expect(page.locator('.grid__playhead')).toBeVisible();

  await page.mouse.click(box.x + 1, box.y + box.height / 2);

  // Near zero, not exactly zero: a click one pixel in is a real position, and
  // rounding it down to the start would make the first pixel of the grid
  // unseekable. ⚠ Exactly 0 hides the marker entirely (`playhead > 0` is what
  // draws it), which is what Stop produces — that path is asserted in
  // `plugin/src/voice.rs`, where there is a transport to assert it against.
  // Measured, like the test above — the drawn line has to sit at the left edge
  // of the track, not merely hold a small number.
  const line = await page.locator('.grid__playhead').boundingBox();
  if (!line) throw new Error('the playhead should be laid out');
  expect(Math.abs(line.x - box.x)).toBeLessThan(6);
});

/**
 * ⛔ **The four melodic generators, which had no way to move the playhead at
 * all.** The drum grid and the song timeline both seek on click; the piano
 * roll's ruler did not, so TASK-041T's *"in all five generators"* was true of
 * one of them. The strip carries the loop brace as well, so the two gestures
 * are split by whether the pointer moved a snap step.
 *
 * Asserted through the footer's bar/beat readout rather than through a stored
 * variable: the roll draws its marker on a canvas, and reading the value back
 * off the handler that wrote it would stay green with the whole readout
 * unwired. The position display is what a producer actually looks at.
 */
async function openRollAndRuler(page: import('@playwright/test').Page) {
  await page.goto('/');
  const search = page.getByRole('combobox', { name: 'Roster' });
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('tab', { name: 'Melody' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="roll-notes"] li').first()).toBeAttached();

  const ruler = page.locator('[data-testid="roll-ruler"]');
  const box = await ruler.boundingBox();
  if (!box) throw new Error('the ruler should be laid out');
  const roll = page.locator('.roll__canvas');
  const read = async (el: import('@playwright/test').Locator, name: string) =>
    Number(await el.getAttribute(name));
  const gutter = await read(roll, 'data-gutter');
  const zoom = await read(roll, 'data-zoom');
  const scrollTick = await read(roll, 'data-scroll-tick');
  const ppq = await read(roll, 'data-ppq');

  return {
    ruler,
    y: box.y + 4,
    bar: ppq * 4,
    at: (tick: number) => box.x + gutter + ((tick - scrollTick) / ppq) * zoom,
    loopFrom: () => read(ruler, 'data-loop-from'),
    loopTo: () => read(ruler, 'data-loop-to'),
  };
}

test('clicking the roll’s ruler moves the playhead rather than laying down a loop', async ({
  page,
}) => {
  const r = await openRollAndRuler(page);
  const position = page.locator('.transport__position');
  await expect(position).toHaveText('1.1.00');

  const before = { from: await r.loopFrom(), to: await r.loopTo() };

  // Bar 3 of a four-bar clip — halfway, so the readout is unambiguous.
  await page.mouse.click(r.at(r.bar * 2), r.y);

  await expect(position).toHaveText(/^3\.1\./);

  // ⛔ And the brace did not move. A click used to lay down a one-step loop
  // (`regionBetween(t, t)` floors the width at one snap step), which is a
  // region the producer never asked for sitting on their clip.
  expect(await r.loopFrom()).toBe(before.from);
  expect(await r.loopTo()).toBe(before.to);
});

test('a drag on the ruler still sets a loop, and does not seek', async ({ page }) => {
  const r = await openRollAndRuler(page);
  const position = page.locator('.transport__position');

  await page.mouse.move(r.at(r.bar), r.y);
  await page.mouse.down();
  await page.mouse.move(r.at(r.bar * 3), r.y, { steps: 8 });
  await page.mouse.up();

  expect(await r.loopFrom()).toBe(r.bar);
  expect(await r.loopTo()).toBe(r.bar * 3);
  // The gesture that draws a brace is not also a seek, or every loop the
  // producer set would rewind the transport under them.
  await expect(position).toHaveText('1.1.00');
});

/**
 * ⛔⛔ **Where the transport lives, which changed on 2026-08-06.** Mike: *"the
 * play/pause and stop buttons and loop button need to be moved to the top of the
 * app to the right of the generators tabs, so that way you can play the
 * generators from there."*
 *
 * ⚠ Pinned as *position*, not as existence: the buttons were always there and
 * every existing spec passed with them in the footer, so nothing would have
 * caught them sliding back down.
 */
test('play, stop and loop sit at the top, above the generator tabs', async ({ page }) => {
  const header = page.locator('.stage__header');
  const controls = header.locator('.transport__controls');
  await expect(controls).toHaveCount(1);

  for (const name of ['Play', 'Stop', 'Loop']) {
    await expect(controls.getByRole('button', { name, exact: true })).toHaveCount(1);
    // ⛔ And nowhere else — two Play buttons is worse than one in the wrong
    // place. ⚠ `exact`, because each drum pad now carries a "Play Kick" button
    // and a substring match finds all eight of them.
    await expect(page.getByRole('button', { name, exact: true })).toHaveCount(1);
  }

  // ⛔ **ABOVE the tabs, not to their right** — Mike, 2026-08-09: *"move these
  // buttons to the first row and have the tabs on the second row above the piano
  // roll"*, then *"center those buttons"*. The header is two rows now, and the
  // reason is legibility: one row carrying six tabs, the take history and three
  // transport buttons had to scroll the tabs on any window narrower than the
  // layout — worst exactly when a producer needs to see which generator they are
  // on.
  //
  // ⚠ Still pinned as *position* rather than existence, for the reason the old
  // assertion gave: the buttons were always there, so nothing would catch them
  // sliding back into the footer.
  const tabs = await page.locator('.tabs').boundingBox();
  const box = await controls.boundingBox();
  expect(tabs && box).toBeTruthy();
  if (!tabs || !box) return;
  expect(box.y).toBeLessThan(tabs.y);

  // The footer keeps everything that is not transport.
  await expect(page.locator('.transport .transport__controls')).toHaveCount(0);
  await expect(page.locator('.transport .meter')).toHaveCount(1);
});

/**
 * ⛔⛔ **The Loop button is a real toggle now.** Mike, 2026-08-06: *"can you have
 * the 'Loop' button toggle off and on and either loop every time it plays to the
 * end of the 4 or 8 bars or stop at the end of the 4 or 8 bars…and can you have
 * it toggled a different background color?"*
 *
 * ⚠ Whether the clip actually repeats is the schedule's, and
 * `voice.rs::transport_tests` owns it —
 * `the_loop_button_repeats_the_whole_clip_when_no_brace_was_dragged` and
 * `switching_the_loop_button_off_runs_to_the_end_and_stays_there`. What only a
 * browser shows is that the control is live and that its state is *visible*
 * rather than only announced.
 */
test('Loop toggles, and shows it with a background rather than only to a screen reader', async ({
  page,
}) => {
  const loop = page.getByRole('button', { name: 'Loop' });

  // ⛔ It used to be `disabled` with a permanent `aria-pressed` — the last dead
  // control in the app, which is TASK-137's complaint.
  await expect(loop).toBeEnabled();
  await expect(loop).toHaveAttribute('aria-pressed', 'true');

  const lit = await loop.evaluate((el) => getComputedStyle(el).backgroundColor);

  await loop.click();
  await expect(loop).toHaveAttribute('aria-pressed', 'false');
  const dark = await loop.evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(dark).not.toBe(lit);

  await loop.click();
  await expect(loop).toHaveAttribute('aria-pressed', 'true');
});
