import { expect, test } from '@playwright/test';

/**
 * Original Workflow — build a style of your own, save it, generate from it
 * (TASK-040U).
 *
 * ⛔ **What a browser can prove here, and what it cannot.** The store, its slug
 * rule, its refusal of a shipped id and its resolution against `extends` are
 * Rust and are tested in `plugin/src/models.rs` — the mock keeps saved styles in
 * memory for one page load and says so. What only this can prove is the screen:
 * that the way in is *reachable without scrolling*, that saving puts a row in
 * the roster marked as the producer's own, that reopening shows what was saved,
 * and that the scale picker offers the base's scales rather than all forty.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('the way in is pinned above everything, whatever is selected', async ({ page }) => {
  // ⛔ Mike's rule: it pins to the top above every artist and producer, always.
  // Asserted by position rather than by existence — a button that is present
  // but below a five-hundred-row roster is the failure the rule is about.
  const original = page.getByRole('button', { name: /Original Workflow/ });
  await expect(original).toBeVisible();

  const rail = page.locator('.rail--left');
  const originalBox = await original.boundingBox();
  const searchBox = await rail.locator('input').first().boundingBox();
  const rosterBox = await rail.locator('.roster__item').first().boundingBox();

  expect(originalBox).not.toBeNull();
  expect(searchBox).not.toBeNull();
  expect(rosterBox).not.toBeNull();
  expect(originalBox!.y).toBeLessThan(searchBox!.y);
  expect(originalBox!.y).toBeLessThan(rosterBox!.y);

  // And it survives selecting something, which is when a rail that reorders
  // itself would lose it.
  await rail.locator('.roster__item').first().click();
  await expect(original).toBeVisible();
});

test('a saved style joins the roster marked as yours, and reopens as itself', async ({
  page,
}) => {
  await page.getByRole('button', { name: /Original Workflow/ }).click();

  const dialog = page.getByRole('dialog', { name: 'Style editor' });
  await expect(dialog).toBeVisible();

  // ⚠ The scale picker offers the *base's* scales, not every scale the engine
  // knows. That is the constraint Mike stated as a rule — an artist generates
  // in a scale that genre uses — arriving at the moment a producer decides what
  // their style is.
  const scales = dialog.locator('.styleeditor__scales input[type="checkbox"]');
  await expect(scales).toHaveCount(2);

  await dialog.getByLabel('Name').fill('My Dark Trap');
  await scales.first().check();
  await dialog.getByRole('button', { name: 'Save style' }).click();

  // Saved, and said so with the name rather than a bare tick.
  await expect(dialog.locator('.styleeditor__saved')).toContainText('My Dark Trap');
  await dialog.getByRole('button', { name: 'Close' }).click();

  // ⛔ In the roster, wearing the badge that says it is the producer's own —
  // and *not* the tier badge it inherited, which would claim a provenance it
  // does not have.
  const row = page.locator('.roster__item', { hasText: 'My Dark Trap' });
  await expect(row).toBeVisible();
  await expect(row.locator('.badge--mine')).toHaveText('Yours');
  await expect(row.locator('.badge--flagship')).toHaveCount(0);
});

test('a new style opens seeded with the beat on screen, not blank', async ({ page }) => {
  // ⛔ "Start from what you liked, not from a blank form" — the bullet the
  // roadmap calls the difference between an editor a producer will use and one
  // they will open once. Generate first, then open the editor: the base must be
  // what made that beat rather than the default.
  await page.locator('.roster__item', { hasText: 'Mock Artist' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });

  await expect(dialog.locator('.styleeditor__combo-button')).toContainText('Mock Artist');
});

test('the base picker is a list this dialog owns, not an OS popup', async ({ page }) => {
  // ⛔ **Mike found this with a screenshot, 2026-08-09.** A native `<select>`
  // popup inside WebView2 is drawn by the OS, at OS scale, positioned against
  // the *window* — with thirty models in it, it filled most of the screen and
  // floated well away from the field it belonged to. Nothing about a native
  // popup is styleable, so the list has to be one the page draws.
  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });

  const combo = dialog.locator('.styleeditor__combo-button');
  await expect(combo).toHaveAttribute('aria-expanded', 'false');
  await combo.click();

  const menu = dialog.locator('.styleeditor__menu');
  await expect(menu).toBeVisible();

  // ⚠ **The same width as the control and bounded in height** — the two things
  // that were wrong. Asserted as geometry rather than as CSS, because what the
  // producer saw was geometry.
  const field = await combo.boundingBox();
  const list = await menu.boundingBox();
  expect(field).not.toBeNull();
  expect(list).not.toBeNull();
  expect(Math.abs(list!.width - field!.width)).toBeLessThan(2);
  expect(list!.height).toBeLessThanOrEqual(260);
  // ...and directly under it, not somewhere else on screen.
  expect(list!.y).toBeGreaterThanOrEqual(field!.y);
  expect(list!.y - (field!.y + field!.height)).toBeLessThan(24);

  await menu.getByRole('option', { name: 'Trap', exact: true }).click();
  await expect(menu).toHaveCount(0);
  await expect(combo).toContainText('Trap');
});

test('training says how far short the kept set is, rather than going dead', async ({
  page,
}) => {
  // ⛔ Mike asked for a floor and for it to be visible: `18 / 30 kept` is
  // something a producer can act on, and a greyed-out button with no
  // explanation is not. The fit, its constraints and the variety gate are Rust
  // and are measured there over a thousand seeds — what a browser can prove is
  // that the count is on screen and that it moves when a take is starred.
  await page.locator('.roster__item', { hasText: 'Mock Artist' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  const star = page.getByRole('button', { name: 'Keep this take for training' });
  await expect(star).toHaveAttribute('aria-pressed', 'false');
  await star.click();
  await expect(star).toHaveAttribute('aria-pressed', 'true');

  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });

  await expect(dialog.locator('.styleeditor__kept')).toContainText('1 / 30');
  await expect(dialog.getByRole('button', { name: 'Train' })).toBeDisabled();
});

test('saving copies no samples unless the producer says so', async ({ page }) => {
  // ⛔⛔ **Mike's instruction, 2026-08-09**: *"ensure that the end user knows
  // that creating their own original artist adds copies of samples to their
  // workflow to their [computer] and ensure that they want to do that before
  // allowing the app to copy the samples."*
  //
  // ⚠ **The assertion that matters is the negative one.** A gate is only tested
  // by checking that nothing happened while it was shut — so the mock records
  // every path the page asked to copy, and this reads it.
  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });

  // It says what it would cost, in files and in megabytes, rather than
  // describing it — "some samples" is not consent.
  const consent = dialog.locator('.styleeditor__samples');
  await expect(consent).toContainText('1 samples');
  await expect(consent).toContainText('MB');

  const box = consent.getByRole('checkbox');
  await expect(box).not.toBeChecked();

  await dialog.getByLabel('Name').fill('No Copies');
  await dialog.getByRole('button', { name: 'Save style' }).click();
  await expect(dialog.locator('.styleeditor__saved')).toContainText('No Copies');

  expect(await page.evaluate(() => window.__freallyCopiedSamples ?? [])).toEqual([]);
});

test('ticking the box is what lets the samples be copied', async ({ page }) => {
  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });

  await dialog.getByLabel('Name').fill('With Copies');
  await dialog.locator('.styleeditor__samples').getByRole('checkbox').check();
  await dialog.getByRole('button', { name: 'Save style' }).click();

  await expect(dialog.locator('.styleeditor__saved')).toContainText('Copied 1 samples');
  expect(await page.evaluate(() => window.__freallyCopiedSamples ?? [])).toEqual([
    'C:/samples/my-kick.wav',
  ]);
});

test('an unnamed style is refused with a reason rather than saved as nothing', async ({
  page,
}) => {
  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });

  await dialog.getByRole('button', { name: 'Save style' }).click();

  await expect(dialog.locator('.styleeditor__error')).toBeVisible();
  // Nothing reached the roster.
  await expect(page.locator('.roster__item .badge--mine')).toHaveCount(0);
});
