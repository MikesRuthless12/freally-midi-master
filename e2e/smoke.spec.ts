import { expect, test } from '@playwright/test';

/**
 * The smoke suite: does the Studio come up, and do its controls respond?
 *
 * These run against `vite dev` with IPC mocked, so they cover the UI layer
 * only. Anything that needs the Rust core belongs in the cargo tests.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  // Every test needs the shell mounted; waiting here keeps each body focused
  // on the behaviour it is actually asserting.
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('the Studio renders every region', async ({ page }) => {
  await expect(page.getByRole('combobox', { name: 'Roster' })).toBeVisible();
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeVisible();
  await expect(page.getByText('Search an artist. Cook.')).toBeVisible();
});

test('all six generator tabs are present', async ({ page }) => {
  // ⚠ **Scoped to the generator tablist.** The sample browser grew its own
  // tablist on 2026-08-10 — up to eight library folders — so an unscoped
  // `getByRole('tab')` counts those too and this read seven.
  const tabs = page.getByRole('tablist', { name: 'Generator' }).getByRole('tab');
  await expect(tabs).toHaveCount(6);
  await expect(tabs).toHaveText(['Drums', 'Melody', 'Counter', 'Bass', 'Chords', 'Song']);
});

test('switching tabs moves the selection', async ({ page }) => {
  await expect(page.getByRole('tab', { name: 'Drums' })).toHaveAttribute(
    'aria-selected',
    'true',
  );

  await page.getByRole('tab', { name: 'Chords' }).click();

  await expect(page.getByRole('tab', { name: 'Chords' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(page.getByRole('tab', { name: 'Drums' })).toHaveAttribute(
    'aria-selected',
    'false',
  );
});

test('controls that cannot work yet are disabled rather than merely inert', async ({
  page,
}) => {
  // A control that looks live but does nothing is worse than one that admits
  // it, and screen readers need to be told.
  //
  // The rule has not changed since Phase 0; what each control can do has.
  //
  // ⛔ **Generate is disabled until somebody is chosen, and it is meant to be.**
  // An auto-select briefly removed this state on 2026-08-09 and was reverted the
  // next day: it took the landing screen with it. Mike, 2026-08-10: *"ensure
  // that they have to pick an artist before they ever even generate anything,
  // because I LOVE that landing screen."*
  await expect(page.getByRole('combobox', { name: 'Roster' })).toBeEnabled();
  await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Stop' })).toBeDisabled();
});

test('K toggles the right rail', async ({ page }) => {
  // ⚠ The KIT panel's *header*, not any button named after it: the panel gained
  // a "Save kit" control (TASK-051), and an unscoped /Kit/i locator resolves to
  // two elements whenever the panel is expanded.
  const kit = page.locator('.rail__title', { hasText: /Kit/i });
  await expect(kit).toBeVisible();

  await page.keyboard.press('k');
  await expect(kit).toBeHidden();

  await page.keyboard.press('k');
  await expect(kit).toBeVisible();
});

test('a rail swaps groups from its tab and remembers it across a reload', async ({ page }) => {
  // ⛔⛔ **INVERTED 2026-08-11: panels do not collapse, rails swap groups.** This
  // read "a panel collapses from its header and stays collapsed across a
  // reload", clicking the GENRES header and asserting `aria-expanded`. Mike
  // replaced the accordion — *"only leave 2 open at a time … file explorer's
  // vertical tab replaces and takes the place of both roster and genres"* — so
  // there is no header toggle left. The half worth keeping is that the choice
  // survives a relaunch, which is what this now drives.
  await expect(page.locator('.rail__title', { hasText: /Roster/i })).toBeVisible();

  // The tab names what it will bring, not what is showing.
  await page.locator('.railtabs__tab', { hasText: /Browser/i }).click();

  await expect(page.locator('.rail__title', { hasText: /Browser/i })).toBeVisible();
  await expect(page.locator('.rail__title', { hasText: /Roster/i })).toHaveCount(0);

  await page.reload();
  await expect(page.locator('.rail__title', { hasText: /Browser/i })).toBeVisible();
});

test('the View menu lists every panel', async ({ page }) => {
  await page.getByRole('button', { name: /View/i }).click();
  const items = page.getByRole('menuitemcheckbox');
  // The right rail plus one per `SECTIONS` in `src/state/ui.ts`: genres, roster,
  // browser, kit, stems, session, presets, pattern library. The menu is built
  // from that list, so adding a panel is meant to land here — this count is the
  // reminder to check the new one actually appears rather than a number to bump.
  await expect(items).toHaveCount(9);
  // ⚠ And the newest one by name, which is what the count is a reminder to do.
  // A panel the View menu cannot reach is one a producer who collapsed it
  // cannot get back.
  await expect(page.getByRole('menuitemcheckbox', { name: 'Pattern library' })).toHaveCount(1);
  await expect(items.first()).toContainText('Right rail');
  // ⚠ The last row is the newest panel, which is the pattern library since
  // TASK-045A — the menu is built from `SECTIONS` in order, so "last" tracks
  // that list rather than naming a panel that happened to be at the end.
  await expect(items.last()).toContainText('Pattern library');
  // ⚠ Named, not just counted. `Stems` arrived with TASK-131F and had to go in
  // the rail rather than the stage toolbar — a control in `stage__controls`
  // costs the velocity lane height and fails `piano-roll.spec.ts`, which is how
  // this panel ended up here.
  await expect(page.getByRole('menuitemcheckbox', { name: /Stems/i })).toBeVisible();
  // ...and `Browser` arrived with TASK-132, in the *left* rail for the opposite
  // reason: it is a place you go before generating rather than after, and the
  // panel that gets widened cannot be the one sharing a column with the kit.
  await expect(page.getByRole('menuitemcheckbox', { name: /Browser/i })).toBeVisible();
});

test('the theme toggle switches the document theme', async ({ page }) => {
  const html = page.locator('html');

  await page.getByRole('button', { name: 'Light theme' }).click();
  await expect(html).toHaveAttribute('data-theme', 'light');

  await page.getByRole('button', { name: 'Dark theme' }).click();
  await expect(html).toHaveAttribute('data-theme', 'dark');

  // "System" clears the attribute so CSS can follow the OS on its own.
  await page.getByRole('button', { name: 'Match system theme' }).click();
  await expect(html).not.toHaveAttribute('data-theme', /.*/);
});

test('the app renders without console errors', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`);
  });

  await page.reload();
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await page.getByRole('tab', { name: 'Song' }).click();

  expect(errors).toEqual([]);
});
