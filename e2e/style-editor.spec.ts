import { expect, test } from '@playwright/test';
import { pickArtist, rosterBox } from './app';

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

  // ⚠ **Against the comboboxes, not a roster row** — the search box and the
  // five-hundred-row list were replaced on 2026-08-09, so what "above
  // everything" means now is: above both pickers.
  const rail = page.locator('.rail--left');
  const originalBox = await original.boundingBox();
  const genreBox = await rail.getByRole('combobox', { name: 'Genres' }).boundingBox();
  const rosterBox = await rail.getByRole('combobox', { name: 'Roster' }).boundingBox();

  expect(originalBox).not.toBeNull();
  expect(genreBox).not.toBeNull();
  expect(rosterBox).not.toBeNull();
  expect(originalBox!.y).toBeLessThan(genreBox!.y);
  expect(originalBox!.y).toBeLessThan(rosterBox!.y);

  // And it survives selecting something, which is when a rail that reorders
  // itself would lose it.
  await pickArtist(page, 'Mock Artist');
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
  // ⚠ **In the roster combobox**, since the list it used to join was replaced
  // on 2026-08-09. The badge does the same job it always did — it is the only
  // thing on the row that says whose the style is, now that there are no
  // headings above the entries.
  // ⚠ **The option is waited for rather than typed-and-committed**, because the
  // roster refreshes over the bridge after the save: committing immediately
  // matched nothing and correctly fell back to the previous selection. An
  // `expect` retries, which is what gives the refresh time to land.
  const roster = rosterBox(page);
  await roster.click();
  await roster.fill('My Dark');
  const row = page
    .locator('.combo__menu')
    .getByRole('option')
    .filter({ hasText: 'My Dark Trap' })
    .first();
  await expect(row).toBeVisible();
  // The badge says whose the style is — the only thing on the row that can,
  // now that there are no headings above the entries.
  await expect(row.locator('.combo__badge')).toHaveText('Yours');

  await row.click();
  await expect(roster).toHaveValue('My Dark Trap');
});

test('a new style opens seeded with the beat on screen, not blank', async ({ page }) => {
  // ⛔ "Start from what you liked, not from a blank form" — the bullet the
  // roadmap calls the difference between an editor a producer will use and one
  // they will open once. Generate first, then open the editor: the base must be
  // what made that beat rather than the default.
  await pickArtist(page, 'Mock Artist');
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
  await pickArtist(page, 'Mock Artist');
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

/**
 * The four blocks the editor could not reach (TASK-040U, closed 2026-08-15).
 *
 * ⛔ **The entry stayed ◐ for a stated reason**: *"they cannot yet reach roll
 * vocabulary, snare placement, 808 behaviour or progression families."* Those
 * four are most of what separates one authored style from another, so a producer
 * building a vibe by hand could set a tempo and a scale and nothing that decides
 * how the beat is actually written.
 *
 * ⚠ **What only a browser proves here is the round trip**, which is the half that
 * silently rots: `modelFrom` writing a key the reopened `draftFrom` does not read
 * back leaves a control that forgets what the producer chose. The write itself is
 * `plugin/src/models.rs` and is tested there.
 */
test('the snare, the rolls, the 808 and the progressions save and reopen', async ({ page }) => {
  await page.getByRole('button', { name: /Original Workflow/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Style editor' });
  await expect(dialog).toBeVisible();

  const snare = dialog.locator('fieldset').filter({ hasText: 'Snare' });
  const rolls = dialog.locator('fieldset').filter({ hasText: 'Rolls' });
  const bass = dialog.locator('fieldset').filter({ hasText: '808' });
  const progressions = dialog.locator('fieldset').filter({ hasText: 'Progressions' });

  // ⛔ **Every group starts on "From the base"**, because an authored value
  // replaces the parent's — a control with no unset state would make opening the
  // dialog enough to overwrite what the style is based on.
  await expect(snare.getByRole('radio', { name: 'From the base' })).toBeChecked();
  await expect(bass.getByRole('radio', { name: 'From the base' })).toBeChecked();
  await expect(rolls.getByRole('checkbox', { checked: true })).toHaveCount(0);
  await expect(progressions.getByRole('checkbox', { checked: true })).toHaveCount(0);

  // ⚠ The slide is dead until the 808 has a role: a slide probability with no
  // role is half an 808, and the disabled state is what says so.
  await expect(bass.getByRole('slider')).toBeDisabled();

  await dialog.getByLabel('Name').fill('My Bounce');
  await snare.getByRole('radio', { name: '2 & 4' }).check();
  await rolls.getByRole('checkbox', { name: '16T' }).check();
  await bass.getByRole('radio', { name: 'Answers the bassline' }).check();
  await progressions.getByRole('checkbox', { name: 'i–VI–VII' }).check();
  await expect(bass.getByRole('slider')).toBeEnabled();

  await dialog.getByRole('button', { name: 'Save style' }).click();
  await expect(dialog.locator('.styleeditor__saved')).toContainText('My Bounce');
  await dialog.getByRole('button', { name: 'Close' }).click();

  // Reopen the saved style and every one of the four comes back as it was.
  const roster = rosterBox(page);
  await roster.click();
  await roster.fill('My Bounce');
  const row = page
    .locator('.combo__menu')
    .getByRole('option')
    .filter({ hasText: 'My Bounce' })
    .first();
  await expect(row).toBeVisible();
  await row.click();

  // ⚠ **The pencil, not "Original Workflow".** That button opens a *blank* form
  // — it is the way in to a new style — and the first cut of this test used it
  // and correctly found nothing checked. Editing a saved style is the row's own
  // control, which appears only once the selection is the producer's own.
  await page.getByRole('button', { name: 'Edit My Bounce' }).click();
  await expect(dialog).toBeVisible();
  await expect(snare.getByRole('radio', { name: '2 & 4' })).toBeChecked();
  await expect(rolls.getByRole('checkbox', { name: '16T' })).toBeChecked();
  await expect(bass.getByRole('radio', { name: 'Answers the bassline' })).toBeChecked();
  await expect(progressions.getByRole('checkbox', { name: 'i–VI–VII' })).toBeChecked();

  // ...and the ones never touched are still inheriting rather than authored.
  await expect(rolls.getByRole('checkbox', { name: '32' })).not.toBeChecked();
  await expect(
    progressions.getByRole('checkbox', { name: 'i–iv', exact: true }),
  ).not.toBeChecked();
});
