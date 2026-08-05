import { expect, test } from '@playwright/test';

/**
 * The loop the product exists for (US-001, TASK-028): search someone, generate,
 * see the beat.
 *
 * Runs against `vite dev` with IPC mocked, so it proves the *UI* half — the
 * search finds the right model, Generate reaches the command with what the user
 * chose, and the grid draws what comes back. The engine's half is proved by the
 * cargo tests; this is the part neither of those can see.
 */

/** Where the ripple test parks what it saw, since the sweep outlives no poll. */
type RippleProbe = Window & { __rippleRan?: boolean };

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('type, Enter, Generate — the grid fills', async ({ page }) => {
  const search = page.getByLabel('Search an artist');
  await search.fill('uk');

  // The autosuggest is a combobox, so the list is addressable by role.
  const options = page.getByRole('option');
  await expect(options.first()).toContainText('UK Drill');

  await search.press('Enter');
  await expect(page.getByText('UK Drill is ready. Hit Generate.')).toBeVisible();

  const generate = page.getByRole('button', { name: 'Generate', exact: true });
  await expect(generate).toBeEnabled();
  await generate.click();

  // The grid is a table of lanes; the mock returns kick, snare and hats.
  const grid = page.getByRole('table', { name: 'Generated pattern' });
  await expect(grid).toBeVisible();
  await expect(grid.getByRole('rowheader')).toHaveText(['Closed hat', 'Snare', 'Kick']);

  // Four bars of 16ths is 64 columns per lane, and cells that are *on* have to
  // actually exist — an all-empty grid would satisfy a visibility check alone.
  await expect(grid.getByRole('cell')).toHaveCount(64 * 3);
  await expect(grid.locator('.grid__cell--on')).not.toHaveCount(0);
  await expect(page.getByText(/4 bars · 64 steps · \d+ notes/)).toBeVisible();
});

test('Generate is refused until someone is chosen', async ({ page }) => {
  // Disabled for real, not merely styled: a producer tabbing to it must be
  // told, and a click that silently does nothing is the worst answer.
  await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeDisabled();
  await expect(page.getByText('Search an artist. Cook.')).toBeVisible();
});

test('the arrow keys move the highlight and Enter takes it', async ({ page }) => {
  const search = page.getByLabel('Search an artist');
  await search.fill('a');

  const options = page.getByRole('option');
  await expect(options.first()).toHaveAttribute('aria-selected', 'true');

  await search.press('ArrowDown');
  await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(options.first()).toHaveAttribute('aria-selected', 'false');

  // Wrapping at the ends, so holding a key never dead-ends.
  await search.press('ArrowUp');
  await search.press('ArrowUp');
  await expect(options.last()).toHaveAttribute('aria-selected', 'true');

  const chosen = await options.last().locator('.suggest__name').textContent();
  await search.press('Enter');
  await expect(search).toHaveValue(chosen ?? '');
});

test('Escape closes the suggestions without choosing anything', async ({ page }) => {
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await expect(page.getByRole('option').first()).toBeVisible();

  await search.press('Escape');
  await expect(page.getByRole('option')).toHaveCount(0);
  // Nothing was selected, so the stage still asks for someone.
  await expect(page.getByText('Search an artist. Cook.')).toBeVisible();
});

test('a query that matches nothing says so rather than showing an empty box', async ({
  page,
}) => {
  await page.getByLabel('Search an artist').fill('qqqzzz');
  await expect(page.getByText('Nothing matches “qqqzzz”.')).toBeVisible();
});

test('the seed is shown after generating, and can be pasted back', async ({ page }) => {
  // US-004. The seed chip is the whole promise: the number that comes back is
  // the number that reproduces the beat.
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  const seed = page.getByLabel('Seed — paste one to get the same beat');
  await expect(seed).toHaveValue('424242');

  await seed.fill('2024');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(seed).toHaveValue('2024');
});

test('a longer pattern really is longer', async ({ page }) => {
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');

  // Scoped to the bar picker: the kit pad grid in the right rail has an "8" too.
  await page
    .getByRole('group', { name: 'Pattern length in bars' })
    .getByRole('button', { name: '8' })
    .click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  await expect(page.getByText(/8 bars · 128 steps/)).toBeVisible();
});

test('playback is honestly disabled when there is no device', async ({ page }) => {
  // The mock reports no audio backend, which is the truth in a browser. The
  // transport must say so rather than offering a Play button that fails.
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  const play = page.getByRole('button', { name: 'Play' });
  await expect(play).toBeDisabled();
  await expect(play).toHaveAttribute('title', /Playback is unavailable/);
});

test('choosing someone else clears the pattern that was on screen', async ({ page }) => {
  // The most convincing wrong thing this UI could do is leave one artist's
  // beat under another artist's name.
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();

  await search.fill('uk');
  await search.press('Enter');
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toHaveCount(0);
  await expect(page.getByText('UK Drill is ready. Hit Generate.')).toBeVisible();
});

test('every tab generates, Song included', async ({ page }) => {
  // ⛔ **There is no phase line left, and that is the change TASK-063A/B made.**
  // This test previously asserted the opposite — that Song said "a later phase"
  // and its Generate was disabled — which was the truth right up until Song Mode
  // existed. It is kept pointed at the same question so the day a tab stops
  // generating is a failure here rather than a surprise in a DAW.
  //
  // Song is still not a `Part`: it goes through `useSong` and draws an
  // arrangement rather than an editor, which is why it is asserted separately.
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');

  for (const tab of ['Melody', 'Counter', 'Bass', 'Chords']) {
    await page.getByRole('tab', { name: tab }).click();
    await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeEnabled();
  }

  await page.getByRole('tab', { name: 'Song' }).click();
  await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();
});

test('a melodic part draws in the piano roll, not the drum grid', async ({ page }) => {
  // The four melodic parts were reachable through the bridge and had no editor
  // at all before TASK-041 — `CenterStage` gated them on "arrives in a later
  // phase" because `DrumGrid` was the only renderer.
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');

  await page.getByRole('tab', { name: 'Melody' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();

  // The roll's accessible note list is the canvas's text alternative and the
  // seam every note assertion goes through — see PianoRoll.tsx.
  const notes = page.getByTestId('roll-notes');
  await expect(notes).toBeAttached();
  await expect(notes.locator('li').first()).toBeAttached();

  // And the drum grid is not what is on screen.
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toHaveCount(0);
});

test.describe('generation FX', () => {
  test('the ripple runs over the grid', async ({ page }) => {
    // FR-017: the animation masks generation latency. The canvas is the ripple
    // path, and `data-running` is set for as long as the sweep lasts — at
    // least 300 ms, which is the floor that makes it register at all.
    //
    // ⛔ The FX's *own* canvas, by class. "Any canvas under the host" was only
    // ever the same thing by accident, and stopped being so the moment the
    // editors below grew one of their own.
    const host = page.getByTestId('genfx');
    await expect(host).toHaveClass(/genfx--ripple/);
    await expect(host.locator('.genfx__canvas')).toHaveCount(1);

    // ⛔ Record the attribute rather than polling for it. A mock generation
    // lands in a few milliseconds, so the sweep is the floor — `durationFor`
    // gives it ~360 ms and then it clears itself. `toHaveAttribute` polls, and
    // on a loaded runner the first poll can arrive after the ripple is already
    // over: the test then reports "no animation" about an animation that ran.
    // The observer goes in before the click and cannot miss the window.
    await page.evaluate(() => {
      const node = document.querySelector('[data-testid="genfx"]');
      if (!node) throw new Error('no genfx host to observe');
      (window as RippleProbe).__rippleRan = false;
      new MutationObserver(() => {
        if (node.getAttribute('data-running') === 'true') {
          (window as RippleProbe).__rippleRan = true;
        }
      }).observe(node, { attributes: true, attributeFilter: ['data-running'] });
    });

    const search = page.getByLabel('Search an artist');
    await search.fill('trap');
    await search.press('Enter');
    await page.getByRole('button', { name: 'Generate', exact: true }).click();

    // The sweep ran...
    await expect
      .poll(() => page.evaluate(() => (window as RippleProbe).__rippleRan))
      .toBe(true);

    // ...and it clears itself rather than sitting over the grid for good.
    await expect(host).not.toHaveAttribute('data-running', 'true', { timeout: 2000 });
  });

  test('reduced motion crossfades instead, and never mounts a canvas', async ({ page }) => {
    // The AC for this task. "No motion" is a different path, not a faster one:
    // an object sweeping the screen is exactly what the setting is asking to
    // be spared, so there is nothing to sweep with.
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.reload();

    const host = page.getByTestId('genfx');
    await expect(host).toHaveClass(/genfx--crossfade/);
    await expect(host.locator('.genfx__canvas')).toHaveCount(0);

    const search = page.getByLabel('Search an artist');
    await search.fill('trap');
    await search.press('Enter');
    await page.getByRole('button', { name: 'Generate', exact: true }).click();

    await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();
    await expect(host.locator('.genfx__canvas')).toHaveCount(0);
  });
});

test('the Settings toggle suppresses the ripple without the OS setting', async ({ page }) => {
  // FR-017's other half: the OS setting is honoured on its own, but a machine
  // that does not expose one — or a user who wants everything else animated —
  // needs a switch of their own. Ticking it must take effect immediately, not
  // on the next launch.
  const host = page.getByTestId('genfx');
  await expect(host).toHaveClass(/genfx--ripple/);

  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('tab', { name: 'Appearance' }).click();
  await page.getByRole('checkbox', { name: 'Reduce motion' }).check();
  await page.getByRole('button', { name: 'Close' }).last().click();

  await expect(host).toHaveClass(/genfx--crossfade/);
  await expect(host.locator('.genfx__canvas')).toHaveCount(0);
});
