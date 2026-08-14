import { expect, test } from '@playwright/test';
import { pickArtist, pickGenre } from './app';

/**
 * Genre and artist browsing (TASK-047, FR-009).
 *
 * ⛔⛔ **The chips and the roster list are gone** (2026-08-09). The rail was a
 * search box, a six-chip genre row and a five-hundred-row list; it is now a
 * genre combobox and a roster combobox with the details pane under them. Mike:
 * *"we won't need the search textbox … it will save us some room."*
 *
 * The roadmap's verify line survives the change and is what this still proves:
 * *"filter by 'drill' lists drill artists + genres"* — one box finds both kinds,
 * and a row says which kind it is.
 *
 * ⚠ **The old `cursor: not-allowed` test went with the chips it guarded.** Its
 * lesson did not: a control can be fully wired, fully asserted and still be
 * telling the producer it is dead, because `click()` consults neither the cursor
 * nor the computed style. If a picker ever grows a disabled-looking state again,
 * that assertion is worth writing back.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('one box finds artists and genres alike, and says which is which', async ({ page }) => {
  // ⛔ **Both kinds in one list, which is what the roster held.** Splitting them
  // across two boxes would have meant a producer had to know whether "UK Drill"
  // was an artist or a genre before they could find it.
  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await roster.fill('uk');

  const options = page.locator('.combo__menu').getByRole('option');
  await expect(options.first()).toContainText('UK Drill');
  // The badge is what tells the two kinds apart, now that there are no headings
  // above them.
  await expect(options.first().locator('.combo__badge')).toHaveText('Genre');
});

test('picking a genre lists the way in and that genre’s names, and nothing else', async ({
  page,
}) => {
  // ⛔⛔ **Mike, 2026-08-12**: *"changing to another genre doesn't change the
  // actual names in the Roster to names within that Genre, it keeps the same
  // names and then shows different genres within the Roster list"*, and then the
  // shape he wanted it in: *"only 1 artist like ny-drill /uk-drill then it should
  // just say 'Original Workflow' & 'Pop Smoke'."* Asserted as the **whole list**
  // rather than as three `hasText` probes, because "and nothing else" is the
  // half that was broken — a probe for what should be there passes just as
  // happily with fifty genres stacked underneath.
  await pickGenre(page, 'Trap');

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();

  // ⚠ A regex for the two names because each row carries its badge in the same
  // text node; "Original Workflow" has no badge and is matched exactly.
  await expect(page.locator('.combo__menu').getByRole('option')).toHaveText([
    'Original Workflow',
    /Mock Artist/,
    /mock Producer/,
  ]);
  // Both of Trap's names, under their own rules — the genre's other kind is not
  // dropped just because the group heading above it says "Artists".
  await expect(page.locator('.combo__separator')).toHaveText(['Artists', 'Producers']);
});

test('a genre nobody works in leaves only the way in, and hides nobody from a query', async ({
  page,
}) => {
  // ⛔ The other half of the same instruction: *"if there is no artist/producer
  // then it should just have 'Original Workflow'."*
  await pickGenre(page, 'UK Drill');

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  const options = page.locator('.combo__menu').getByRole('option');
  await expect(options).toHaveText(['Original Workflow']);

  // ⛔⛔ **Enter must do nothing here.** "Original Workflow" is an *action*, and
  // it is now the only row — so the highlight the list opens with is the one
  // thing that may never land on it. It did, for exactly as long as `Combo`
  // clamped its starting index to 0: opening this box and pressing Enter threw
  // the style editor over the whole app.
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: 'Style editor' })).toHaveCount(0);

  // ⛔⛔ **The line the narrowing is allowed to reach and no further.** The rule
  // it replaced — the comboboxes offer the whole roster, never the cross-filtered
  // one — was written against a real defect: hiding entries in a control that is
  // searched by *typing* stops them being found at all. Narrowing what is
  // *browsed* keeps the genre box meaningful; narrowing what a query reaches
  // would make choosing a genre a way to lose the roster.
  await roster.click();
  await roster.fill('mock');
  await expect(options.filter({ hasText: 'Mock Artist' })).toHaveCount(1);
});

test('the roster reads Original Workflow, then Artists, then Producers', async ({ page }) => {
  // ⛔⛔ **Mike, 2026-08-12**: *"put 'Original Workflow' then 'Artists'
  // underlined and then list all artists in alphabetical order and then
  // 'Producers' underlined and then put producers in alphabetical order."*
  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();

  // ⛔ **The separators are NOT options, which is the assertion and not an
  // implementation detail.** A listbox reports its own size — "2 of 12" is read
  // aloud — so a separator counted among the options misstates how many choices
  // there are to everyone who cannot see the rule.
  await expect(page.locator('.combo__menu').getByRole('option')).toHaveText([
    'Original Workflow',
    /Mock Artist/,
    /mock Producer/,
  ]);
  await expect(page.locator('.combo__separator')).toHaveText(['Artists', 'Producers']);
});

test('a separator is never what typing its word lands on', async ({ page }) => {
  // ⛔⛔ **Mike, 2026-08-12**: *"ensure that when you type 'Artists', that it
  // doesn't let you add that as a roster item"*, and then, correcting a version
  // of this that had reserved the words outright: *"no i don't want it to select
  // the word 'Artists' or the word 'Producers' not the actual artist/producer
  // themselves."* So the query still searches — landing on a real name the
  // matcher considers close is the search working — and what is asserted is that
  // the **separator itself** is never offered and never committed.
  const roster = page.getByRole('combobox', { name: 'Roster' });

  for (const word of ['Artists', 'Producers']) {
    await roster.click();
    await roster.fill(word);

    // Not among the suggestions, and no rule is drawn in a typed list at all.
    await expect(page.locator('.combo__menu').getByRole('option', { name: word })).toHaveCount(
      0,
    );
    await expect(page.locator('.combo__separator')).toHaveCount(0);

    await page.keyboard.press('Enter');
    // Whatever it did or did not settle on, it did not settle on the word.
    await expect(roster).not.toHaveValue(word);
  }
});

test('picking a genre leaves the details clean of a filter notice', async ({ page }) => {
  // ⛔⛔ **INVERTED 2026-08-11.** This read `toContainText('Trap')` against
  // `.roster__filter`, and then pressed "Show all" for the way back. Mike:
  // *"I also don't want the 'Filtered by DrakeShow all' to show up at all in the
  // details part of the roster."* `e2e/cross-filter.spec.ts` carries the full
  // reasoning; this is the second door onto the same removed control and is kept
  // as a refusal so reinstating it fails in both places.
  await pickGenre(page, 'Trap');

  await expect(page.locator('.roster__filter')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Show all' })).toHaveCount(0);
});

test('the pane says what the selection tends to do, before you generate', async ({ page }) => {
  await pickArtist(page, 'Mock Artist');

  const pane = page.locator('.artistpane');
  await expect(pane).toBeVisible();
  await expect(pane.locator('.artistpane__name')).toContainText('Mock Artist');
  // ⚠ The tempo comes from the *model's* defaults, not from the pins — a pane
  // showing the pins would describe an artist who plays neither.
  await expect(pane.locator('.artistpane__tends').first()).toContainText('BPM');
});

/**
 * ⛔⛔ **The half of TASK-158D that prevents silence.**
 *
 * Mike: *"The detail pane should tell a producer what they are about to get —
 * genres, moods, tempo range, what the model does and does **not** cover —
 * rather than making them press Generate to find out."*
 *
 * `bass.rs`, `chords.rs`, `melody.rs` and `counter.rs` each return an **empty**
 * track when the model authored no block of their own. That is right — an artist
 * who does not write countermelodies should not have one invented for them — but
 * it means Generate on that tab produces nothing at all, and before this the only
 * way to learn that was to press it. `engine/tests/coverage.rs` proves the claim
 * by generating every shipped model; this proves the pane actually says it.
 */
test('the pane says what the selection does NOT write, not only what it does', async ({
  page,
}) => {
  await pickArtist(page, 'Mock Artist');
  const pane = page.locator('.artistpane');

  // ⚠ **A range, not a single tempo** — an artist at 68–96 and one at 138–142 are
  // different propositions at the same nominal.
  await expect(pane).toContainText('132–148 BPM');
  // Named rather than counted: "2 moods" says there is a control, not what the
  // artist is.
  await expect(pane).toContainText('dark · bounce');

  await expect(pane.locator('.artistpane__tends').filter({ hasText: 'Writes' })).toContainText(
    'Drums · Chords · Melody · Bass',
  );
  await expect(pane.locator('.artistpane__missing')).toContainText('Counter');
});

test('the pane follows the selection rather than sticking to the first thing read', async ({
  page,
}) => {
  await pickArtist(page, 'Mock Artist');
  await expect(page.locator('.artistpane__name')).toContainText('Mock Artist');

  await pickGenre(page, 'Trap');
  await expect(page.locator('.artistpane__name')).toContainText('Trap');
});

test('“Original Workflow” is reachable whatever genre is selected', async ({ page }) => {
  // ⛔⛔ **Mike, 2026-08-09**: *"ensure that 'Original Workflow' is at the top of
  // the artist/producer combobox no matter which genre is selected, so that way
  // you can always start an original artist/producer workflow and save it."* It
  // is the way in to building your own, so no filter may hide it.
  await pickGenre(page, 'UK Drill');

  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();

  const options = page.locator('.combo__menu').getByRole('option');
  await expect(options.first()).toContainText('Original Workflow');
});

test('choosing Original Workflow opens the editor rather than selecting a style', async ({
  page,
}) => {
  // ⛔ It is an *action*, not a choice. Nothing may land on it by accident —
  // when it was an ordinary option the never-blank fallback chose it, blur
  // committed it, and the style editor opened over the whole app.
  const roster = page.getByRole('combobox', { name: 'Roster' });
  await roster.click();
  await page.locator('.combo__menu').getByRole('option').first().click();

  await expect(page.getByRole('dialog', { name: 'Style editor' })).toBeVisible();
});
