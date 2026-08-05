import { expect, test } from '@playwright/test';

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
  await page.locator('.roster__item', { hasText: 'Mock Artist' }).click();
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
