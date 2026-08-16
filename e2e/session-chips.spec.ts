import { expect, test } from '@playwright/test';

import { openPanel, pickArtist } from './app';

/**
 * The session chips and the keep-or-adopt prompt (TASK-033, FR-002).
 *
 * The store's own rules are covered by `src/state/session.test.ts`. What only a
 * browser can show is that an empty chip really is empty — that the artist's
 * tempo arrives as a *placeholder* and not as a value the user appears to have
 * chosen — and that the prompt stays reachable when the rail holding the chips
 * is not.
 */

const chip = (name: string) => `.session__chip:has-text("${name}")`;

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  // ⛔ The panel is behind a vertical tab now — `openPanel` presses it.
  await openPanel(page, 'session');
});

async function pick(page: import('@playwright/test').Page, query: string) {
  const search = page.getByRole('combobox', { name: 'Roster' });
  await search.fill(query);

  // ⛔ Click the option; do not press Enter. Enter goes through
  // `SearchBar.onKeyDown`, which returns early unless the dropdown is *still*
  // open — and the list closes on blur. Under load the keypress lands on a
  // closed list, selects nothing, and reports nothing: `select()` also returns
  // early when the id has not changed, so a mis-timed Enter is indistinguishable
  // from a successful one. It surfaces much later as "the switch prompt never
  // appeared", which points at the prompt rather than at the selection that
  // never happened — and three different tests in this file have now failed
  // that way on macOS.
  //
  // `onMouseDown` calls `choose` directly and preventDefaults exactly so focus
  // cannot close the list under the click, which makes it the one path with no
  // timing in it. Filtering by the query pins the *right* option, since the
  // previous query's results are still on screen for a frame after `fill`.
  // `dispatchEvent` rather than `click`, and that is the third attempt at this
  // line. `click` first waits for the element to be visible, enabled and
  // stable — and the dropdown unmounts the moment the input blurs, so that wait
  // is itself the window in which the option vanishes: "element was detached
  // from the DOM, retrying", until the test times out. Dispatching resolves the
  // element and fires synchronously, so there is no window to lose.
  await page
    .getByRole('option')
    .filter({ hasText: new RegExp(query, 'i') })
    .first()
    .dispatchEvent('mousedown');

  // `choose` writes the artist's full name back into the box, so a value that is
  // no longer the raw query is the proof the selection actually landed.
  await expect(search).not.toHaveValue(query);
}

test('the chips ask for an artist before they show anything', async ({ page }) => {
  await expect(page.getByText('Pick an artist to see what it asks for.')).toBeVisible();
});

test('the artist’s tempo is a placeholder, not a value', async ({ page }) => {
  // The distinction the whole chip design rests on: "140 because trap says so"
  // must not look like "140 because I typed it". An empty box with a hint is
  // the only way to show the second without claiming the first.
  await pick(page, 'trap');

  const bpm = page.locator(`${chip('BPM')} input`);
  await expect(bpm).toHaveValue('');
  await expect(bpm).toHaveAttribute('placeholder', '140');

  const swing = page.locator(`${chip('Swing')} input`);
  await expect(swing).toHaveValue('');
  await expect(swing).toHaveAttribute('placeholder', '0.54');
});

test('key and scale offer the artist’s choice until a beat exists', async ({ page }) => {
  // A seed picks them, so before Generate there is nothing honest to show.
  await pick(page, 'trap');

  // ⚠ Comboboxes since TASK-057, so this reads the field itself rather than the
  // first `<option>` — which is a better question anyway: what the chip *shows*
  // is the thing a producer reads, and the old assertion could have passed over
  // a control displaying something else entirely.
  const key = page.locator(chip('Key')).getByRole('combobox');
  await expect(key).toHaveValue('The artist’s');

  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();

  // ⛔ **The first Generate pulls the right rail back to `kit · stems`**, on
  // purpose — `useUi.revealStems` shows a producer where their stems went, once.
  // The session chips live in the other group, so they have to be asked for
  // again afterwards. `openPanel` is idempotent, so this is a no-op on any run
  // where the reveal has already been spent.
  await openPanel(page, 'session');

  // ...and afterwards it says which one the artist landed on. The mock
  // generates in F♯ natural minor.
  await expect(key).toHaveValue('F♯ — the artist’s');
  await expect(page.locator(chip('Scale')).getByRole('combobox')).toHaveValue(
    'Natural minor — the artist’s',
  );
});

test('a pinned tempo is kept, and can be handed back', async ({ page }) => {
  await pick(page, 'trap');

  const bpm = page.locator(`${chip('BPM')} input`);
  await bpm.fill('88');
  await bpm.blur();
  await expect(bpm).toHaveValue('88');

  // The unpin button only exists once something is pinned — before that there
  // is nothing to undo.
  const unpin = page.locator(chip('BPM')).getByRole('button', {
    name: 'Back to the artist’s value',
  });
  await unpin.click();
  await expect(bpm).toHaveValue('');
  await expect(unpin).toHaveCount(0);
});

test('a tempo outside the musical range is corrected on the way out', async ({ page }) => {
  // The engine clamps too, and this is why the chip must: a box reading 5 over
  // a beat generated at 20 is a readout that lies. The range is Ableton
  // Live's, 20–999, so that a project the DAW is running at is never a tempo
  // this app refuses to show.
  await pick(page, 'trap');

  const bpm = page.locator(`${chip('BPM')} input`);
  await bpm.fill('5');
  await bpm.blur();
  await expect(bpm).toHaveValue('20');

  // 900 is a legal tempo, not a typo, and must survive untouched.
  await bpm.fill('900');
  await bpm.blur();
  await expect(bpm).toHaveValue('900');
});

test('the tempo box refuses anything that is not a digit', async ({ page }) => {
  // `<input type="number">` looks like it does this and does not — it accepts
  // `e`, `E`, `+`, `-` and `.`, so "1e5" is a legal value that arrives as
  // 100000. The chip is a text input with a digit filter for that reason.
  await pick(page, 'trap');

  const bpm = page.locator(`${chip('BPM')} input`);
  await bpm.fill('12e5');
  await expect(bpm).toHaveValue('125');

  await bpm.fill('abc');
  await expect(bpm).toHaveValue('');

  await bpm.fill('-14.0');
  await expect(bpm).toHaveValue('140');
});

test.describe('keep or adopt', () => {
  test('switching artists with a pinned session asks which one wins', async ({ page }) => {
    await pick(page, 'trap');
    const bpm = page.locator(`${chip('BPM')} input`);
    await bpm.fill('88');
    await bpm.blur();

    await pick(page, 'uk');
    const prompt = page.locator('.switch-prompt');
    await expect(prompt).toBeVisible();
    await expect(prompt).toContainText('UK Drill');
    // The row shows what is actually in dispute, not every field.
    await expect(prompt.locator('li')).toHaveCount(1);
    await expect(prompt.locator('li')).toContainText('88');

    await prompt.getByRole('button', { name: 'Keep mine' }).click();
    await expect(prompt).toHaveCount(0);
    await expect(bpm).toHaveValue('88');
  });

  test('adopting the new artist empties the chips', async ({ page }) => {
    await pick(page, 'trap');
    const bpm = page.locator(`${chip('BPM')} input`);
    await bpm.fill('88');
    await bpm.blur();

    await pick(page, 'uk');
    await page.getByRole('button', { name: 'Use UK Drill’s' }).click();

    await expect(page.locator('.switch-prompt')).toHaveCount(0);
    await expect(bpm).toHaveValue('');
    await expect(bpm).toHaveAttribute('placeholder', '140');
  });

  test('nothing is asked when nothing was pinned', async ({ page }) => {
    await pick(page, 'trap');
    await pick(page, 'uk');
    await expect(page.locator('.switch-prompt')).toHaveCount(0);
  });

  test('the prompt survives the rail that holds the chips being closed', async ({ page }) => {
    // It sits by Generate for exactly this reason: the right rail collapses
    // under 1440px and behind K, and a question nobody can see would leave one
    // artist's tempo quietly attached to another's beat.
    await pick(page, 'trap');
    const bpm = page.locator(`${chip('BPM')} input`);
    await bpm.fill('88');
    await bpm.blur();

    await pick(page, 'uk');
    await page.getByRole('tablist', { name: 'Generator' }).click();
    await page.keyboard.press('k');
    await expect(page.locator('.rail--right')).toHaveCount(0);

    await expect(page.locator('.switch-prompt')).toBeVisible();
    await page.getByRole('button', { name: 'Use UK Drill’s' }).click();
    await expect(page.locator('.switch-prompt')).toHaveCount(0);
  });
});

/**
 * The Simple / Complex switch (TASK-125).
 *
 * ⛔ **Mike asked for one control over all four melodic generators**, moving
 * them together between a plain reading of the style and a busy one. What the
 * engine does with the answer is `engine/tests/complexity.rs`, measured over the
 * shipped roster; what only a browser can show is that the control is on screen,
 * says which state it is in, and that the answer **leaves the page**.
 *
 * ⚠ **Asserted on what was SENT, not on what came back** — the same reasoning
 * `generate-in.spec.ts` gives for the base pin. This mock has no engine, so it
 * cannot answer a busy request with busier notes, and a spec that read only the
 * reply could not tell a chip wired to the request from one wired to nothing.
 */
test('the busy switch starts on the model as written and travels with a generation', async ({
  page,
}) => {
  await pickArtist(page, 'Mock Artist');

  const busy = () => page.getByRole('group', { name: 'Density' });
  await expect(busy()).toBeVisible();

  // ⛔ **Three states with the middle one selected.** `As written` generates
  // byte-for-byte what the app did before this switch existed, which is what
  // makes it safe for every saved seed — two states with no neutral would have
  // made opening the app enough to change what an artist sounds like.
  await expect(busy().getByRole('button', { name: 'As written' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await expect(busy().getByRole('button', { name: 'Plain' })).toHaveAttribute(
    'aria-pressed',
    'false',
  );

  // ⚠ **The switch is set BEFORE generating, and the panel is re-opened after.**
  // The Stems panel reveals itself the moment anything is generated (TASK-131),
  // and the rail shows one section at a time — so the chip is *unmounted* by a
  // Generate, not merely scrolled off. The first cut of this test pressed
  // Generate and then reached for the chip, and spent thirty seconds waiting for
  // an element that no longer existed.
  await busy().getByRole('button', { name: 'Busy', exact: true }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => window.__freallyGeneratedComplexity ?? []))
    .toContain('complex');

  // ...and back again, so the switch is not one-way.
  await openPanel(page, 'session');
  await busy().getByRole('button', { name: 'As written' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => window.__freallyGeneratedComplexity ?? []))
    .toContain('authored');
});
