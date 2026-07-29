import { expect, test } from '@playwright/test';

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
});

async function pick(page: import('@playwright/test').Page, query: string) {
  const search = page.getByLabel('Search an artist');
  await search.fill(query);
  await search.press('Enter');
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

  const key = page.locator(`${chip('Key')} select`);
  await expect(key).toHaveValue('');
  await expect(key.locator('option').first()).toHaveText('The artist’s');

  await page.getByRole('button', { name: 'Generate' }).click();
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();

  // ...and afterwards it says which one the artist landed on. The mock
  // generates in F♯ natural minor.
  await expect(key.locator('option').first()).toHaveText('F♯ — the artist’s');
  await expect(page.locator(`${chip('Scale')} select option`).first()).toHaveText(
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
