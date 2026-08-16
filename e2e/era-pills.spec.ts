import { expect, test } from '@playwright/test';

/**
 * Browsing the roster by when a style was current (TASK-158G).
 *
 * ⛔⛔ **Mike, 2026-08-10:** *"allow the end user to [filter] the list by what
 * genre/artist was out within those specific years instead of trying to search
 * through them all and not finding what you want and just randomly searching for
 * names through genres/artists/producers blindly."*
 *
 * ▶ **This is the half a combobox cannot answer.** Typing works when you can
 * already name the thing; at four hundred names, browsing is what you do when
 * you cannot — and "what was out when I was listening" is the one axis a
 * producer always knows.
 *
 * ⛔ **The line this must not cross is the one the genre cross-filter already
 * draws:** narrowing what you *browse* is a convenience, narrowing what you can
 * *find* is a defect. The last test here is that rule, and it is the one worth
 * keeping if the others ever have to change.
 *
 * ⚠ The parser itself — two dash characters, decades against single years,
 * open-ended spans, the union across pills — is `src/lib/era.test.ts`, against
 * the shipped `data/` rather than a fixture. What this proves is the wiring.
 */

const ERAS = '[data-testid="era-pills"]';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('the four pills start unpressed, so the whole roster browses', async ({ page }) => {
  // ⛔ **Empty means no filter, never "nothing matches".** Pills that hid the
  // roster until one was pressed would make the list look broken in the state it
  // spends most of its life in.
  const pills = page.locator(`${ERAS} button`);
  await expect(pills).toHaveText(['1990s', '2000s', '2010s', '2020s']);
  for (const decade of ['1990s', '2000s', '2010s', '2020s']) {
    await expect(page.getByRole('button', { name: decade })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  }

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await expect(page.locator('.combo__menu').getByRole('option')).toHaveText([
    'Original Workflow',
    /Mock Artist/,
    /mock Producer/,
  ]);
});

test('a pressed pill keeps the names from those years and drops the rest', async ({ page }) => {
  // Mock Artist is 1994-1999; mock Producer is 2018–present.
  await page.getByRole('button', { name: '1990s' }).click();
  await expect(page.getByRole('button', { name: '1990s' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  const options = page.locator('.combo__menu').getByRole('option');
  await expect(options).toHaveText(['Original Workflow', /Mock Artist/]);

  // ⛔ **And the heading goes with the group.** A "Producers" rule with nobody
  // under it reads as a list that failed to load rather than a group that is
  // empty — the same rule the A–Z grouping already follows.
  await expect(page.locator('.combo__separator')).toHaveText(['Artists']);
});

test('a second pill widens the list rather than narrowing it', async ({ page }) => {
  // ⛔ Multi-select is a union: two pills mean "either". An intersection would
  // make pressing a second pill *remove* names, which is backwards from how
  // every other multi-select in the app reads.
  await page.getByRole('button', { name: '1990s' }).click();
  await page.getByRole('button', { name: '2020s' }).click();

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await expect(page.locator('.combo__menu').getByRole('option')).toHaveText([
    'Original Workflow',
    /Mock Artist/,
    /mock Producer/,
  ]);
});

test('pressing a pill again releases it', async ({ page }) => {
  const nineties = page.getByRole('button', { name: '1990s' });
  await nineties.click();
  await expect(nineties).toHaveAttribute('aria-pressed', 'true');
  await nineties.click();
  await expect(nineties).toHaveAttribute('aria-pressed', 'false');

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await expect(page.locator('.combo__menu').getByRole('option')).toHaveText([
    'Original Workflow',
    /Mock Artist/,
    /mock Producer/,
  ]);
});

test('a pressed pill narrows the genres box as well as the roster', async ({ page }) => {
  // ⛔ Mike's sentence is *"what genre/artist was out"*, and the pills' own note
  // argues its overlap rule with `boom-bap` — a genre.
  //
  // ⚠ **The 2020s, because it is the decade that discriminates.** Trap is
  // `2010s` and UK Drill is `2018–present`, so *both* overlap the 2010s — an
  // assertion there would pass over a filter that did nothing at all. Only the
  // open-ended one reaches the 2020s.
  await page.getByRole('button', { name: '2020s' }).click();

  const genres = page.getByRole('combobox', { name: 'Genres' });
  await genres.click();
  await expect(page.locator('.combo__menu').getByRole('option')).toHaveText(['UK Drill']);
});

test('a pill never makes a GENRE untypable either', async ({ page }) => {
  // ⛔⛔ **This is the one a review caught, and it is the same rule one box
  // over.** The genres box resolved its ranked results by looking each one up in
  // the `options` it was handed — which the pills narrow — so every genre a
  // pressed pill excluded came back `undefined` and was dropped. Pressing the
  // 90s and typing "UK Drill" gave an **empty menu** for a genre that plainly
  // exists, with no "no match" line either.
  await page.getByRole('button', { name: '1990s' }).click();

  const genres = page.getByRole('combobox', { name: 'Genres' });
  await genres.click();
  await genres.fill('UK Drill');
  await expect(
    page.locator('.combo__menu').getByRole('option').filter({ hasText: 'UK Drill' }),
  ).toHaveCount(1);
});

test('a selection a pill excludes still reads in the box', async ({ page }) => {
  // ⛔ **Or the field empties itself under the producer.** `Combo` takes its text
  // from `options`, which the pills narrow — so choosing a genre and then
  // pressing a pill that excludes it blanked the box while that genre was still
  // selected and still cross-filtering the roster below it. `valueLabel` is the
  // fallback that says what is actually held.
  const genres = page.getByRole('combobox', { name: 'Genres' });
  await genres.click();
  await genres.fill('UK Drill');
  await page.locator('.combo__menu').getByRole('option').first().click();
  await expect(genres).toHaveValue('UK Drill');

  await page.getByRole('button', { name: '1990s' }).click();
  await expect(genres).toHaveValue('UK Drill');
});

test('a pill never makes a name untypable', async ({ page }) => {
  // ⛔⛔ **The rule that matters most here.** `LeftRail` states it about the
  // genre cross-filter and it applies unchanged: hiding entries from a control
  // that is searched by *typing* stops them being found at all. So with the 90s
  // pressed, the producer who worked only from 2018 is out of the browse list
  // and still one query away.
  await page.getByRole('button', { name: '1990s' }).click();

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await roster.fill('producer');
  await expect(
    page.locator('.combo__menu').getByRole('option').filter({ hasText: 'mock Producer' }),
  ).toHaveCount(1);
});
