import { expect, test } from '@playwright/test';

/**
 * The phase gate, as far as a browser can check it.
 *
 * `smoke.spec.ts` asks "does the UI work". This asks "is the phase actually
 * done" — the parts of PRD § 7.6 that are assertable without a native build.
 * Run it at the end of every phase: `npm run test:e2e`.
 *
 * What this CANNOT cover is as important as what it can, and every one of
 * those is written up with manual steps in Live-To-Do.md:
 *   - native drag-out into a DAW (no OS drag in a browser context)
 *   - the crash → restart → report loop (needs a real process to kill)
 *   - the updater (needs a signed release to check against)
 *   - audio playback and the audio device
 *   - installers, and the first-run flow on each OS
 *
 * A green run here is necessary, not sufficient. Do not tag a release on it
 * alone.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test.describe('Phase gate — UI contract', () => {
  test('the Studio presents every region the PRD specifies', async ({ page }) => {
    // PRD § 8: left rail, generator tabs, grid stage, right rail, transport.
    await expect(page.getByRole('combobox', { name: 'Roster' })).toBeVisible();
    await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Generate', exact: true })).toBeVisible();
    // ⚠ **The panel *header*, located as one.** This asked for any button
    // matching /Kit/i, which was unique until the KIT panel gained a "Save kit"
    // control (TASK-051) — and then the locator resolved to two elements
    // whenever that panel happened to be expanded, so the gate failed on the
    // state of a collapsed section rather than on anything it is about. The
    // claim is "the right rail is present"; `.rail__toggle` is what says so.
    await expect(page.locator('.rail__title', { hasText: /Kit/i })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  });

  test('the empty state uses the product voice', async ({ page }) => {
    // The vision is explicit about this copy; a generic "No data" would be a
    // brand regression, not a cosmetic one.
    await expect(page.getByText('Search an artist. Cook.')).toBeVisible();
  });

  test('nothing claims to work before it does', async ({ page }) => {
    // Still the rule; the line has moved. Search is live and Generate becomes
    // live once an artist is chosen (TASK-028), so the controls that must stay
    // disabled are the ones with nothing behind them yet: transport with no
    // pattern and no device.
    // ⚠ `exact` matters here: "Generate all" (TASK-120) shares a prefix with
    // "Generate", and Playwright matches an accessible name by substring by
    // default — so the unqualified form resolves to two buttons and fails
    // strict mode rather than asserting anything.
    for (const name of ['Generate', 'Play', 'Stop']) {
      await expect(page.getByRole('button', { name, exact: true })).toBeDisabled();
    }
    await expect(page.getByRole('combobox', { name: 'Roster' })).toBeEnabled();
  });

  test('Loop is live, because it now has something to toggle', async ({ page }) => {
    // ⛔⛔ **INVERTED 2026-08-06, and the inversion is the fix.** Loop was in the
    // sweep above, on the reasoning that it was "a loop toggle that has nothing
    // to toggle" — true while it was permanently pressed and inert. Mike asked
    // for it to work: *"can you have the 'Loop' button toggle off and on and
    // either loop every time it plays to the end of the 4 or 8 bars or stop at
    // the end of the 4 or 8 bars."*
    //
    // ⚠ **Live with nothing generated, deliberately.** It is a mode switch, not
    // an action on a clip — it says what the schedule should do *when* something
    // plays, so there is no state in which setting it early is meaningless.
    // That is exactly why it does not belong in the sweep above, rather than
    // being an exception to it.
    const loop = page.getByRole('button', { name: 'Loop', exact: true });
    await expect(loop).toBeEnabled();
    await expect(loop).toHaveAttribute('aria-pressed', 'true');
  });

  test('a control becomes live exactly when it can do something', async ({ page }) => {
    // The other half of the same rule, and the one a "disabled" sweep cannot
    // see: a control that stays disabled after its precondition is met is
    // just as broken as one that lies about being ready.
    const search = page.getByRole('combobox', { name: 'Roster' });
    const generate = page.getByRole('button', { name: 'Generate', exact: true });

    await expect(generate).toBeDisabled();
    await search.fill('trap');
    await search.press('Enter');
    await expect(generate).toBeEnabled();

    await generate.click();
    await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();
  });
});

test.describe('Phase gate — settings and about', () => {
  test('the transport bar offers Settings and About', async ({ page }) => {
    await expect(page.getByRole('button', { name: 'Settings' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'About' })).toBeVisible();
  });

  test('Settings opens with its categories', async ({ page }) => {
    await page.getByRole('button', { name: 'Settings' }).click();
    const dialog = page.getByRole('dialog', { name: 'Settings' });
    await expect(dialog).toBeVisible();
    await expect(page.getByRole('tab', { name: 'General' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Appearance' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'About' })).toBeVisible();
  });

  test('searching filters the categories', async ({ page }) => {
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.getByLabel('Search settings').fill('dataset');
    await expect(page.getByRole('tab', { name: 'General' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Appearance' })).toHaveCount(0);
  });

  test('Escape closes Settings', async ({ page }) => {
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog', { name: 'Settings' })).toHaveCount(0);
  });

  test('About shows the artist-name disclaimer', async ({ page }) => {
    await page.getByRole('button', { name: 'About' }).click();
    // This text is the product's legal position; it must not silently vanish.
    await expect(page.getByText(/descriptive references to a musical style/)).toBeVisible();
    await expect(page.getByText(/No affiliation, endorsement/)).toBeVisible();
  });
});

test.describe('Phase gate — accessibility', () => {
  test('the core loop is reachable by keyboard alone', async ({ page }) => {
    // PRD § 7: full keyboard operability of the core loop.
    await page.keyboard.press('Tab');
    const focused = await page.evaluate(() => document.activeElement?.tagName);
    expect(focused).not.toBe('BODY');
  });

  test('every tab exposes its selected state', async ({ page }) => {
    const tabs = page.getByRole('tab');
    const count = await tabs.count();
    for (let i = 0; i < count; i++) {
      await expect(tabs.nth(i)).toHaveAttribute('aria-selected', /true|false/);
    }
  });

  test('every panel is reachable from a rail tab', async ({ page }) => {
    // ⛔⛔ **REPLACES "collapsible panels expose aria-expanded"** (2026-08-11).
    // The panels are not collapsible any more — a rail shows one group and the
    // tabs on its edge switch between them, so there is no `aria-expanded` left
    // to assert. The accessibility property that *matters* under the new model
    // is the one this checks: no panel is stranded. Every one of the eight is
    // either on screen or named on a tab that will bring it.
    for (const name of [/Genres/i, /Roster/i, /Browser/i, /Kit/i, /Stems/i, /Session/i]) {
      const showing = page.locator('.rail__title', { hasText: name });
      const offered = page.locator('.railtabs__tab', { hasText: name });
      await expect
        .poll(async () => (await showing.count()) + (await offered.count()))
        .toBeGreaterThan(0);
    }
  });

  test('focus is visible where it lands', async ({ page }) => {
    // A 2px focus ring is specified; assert an outline is actually painted
    // rather than trusting the stylesheet.
    await page.getByRole('tab', { name: 'Melody' }).focus();
    const outline = await page
      .getByRole('tab', { name: 'Melody' })
      .evaluate((el) => getComputedStyle(el).outlineStyle);
    expect(outline).not.toBe('none');
  });
});

test.describe('Phase gate — theming', () => {
  test('both themes apply real colours, not the same one twice', async ({ page }) => {
    const bg = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);

    // The theme swap is animated (140ms in tokens.css), so reading straight
    // after the click samples a colour mid-transition — which is how this test
    // first failed, reporting rgb(110,111,114): the midpoint between the two
    // themes. Poll for the settled value instead of sleeping.
    await page.getByRole('button', { name: 'Dark theme' }).click();
    // The dark theme's charcoal, per PRD § 9.
    await expect.poll(bg).toBe('rgb(11, 12, 16)');
    const dark = await bg();

    await page.getByRole('button', { name: 'Light theme' }).click();
    await expect.poll(bg).toBe('rgb(250, 250, 252)');
    const light = await bg();

    expect(dark).not.toBe(light);
  });

  test('no component hardcodes a colour outside the token system', async ({ page }) => {
    // Every themed surface must move when the theme moves. A panel that stays
    // put is one that hardcoded a hex.
    const sample = async () =>
      page.evaluate(() => {
        const rail = document.querySelector('.rail--left');
        const transport = document.querySelector('.transport');
        return [
          rail ? getComputedStyle(rail).backgroundColor : '',
          transport ? getComputedStyle(transport).backgroundColor : '',
        ];
      });

    await page.getByRole('button', { name: 'Dark theme' }).click();
    // Same transition race as above: wait for the rail to settle first.
    await expect.poll(async () => (await sample())[0]).toBe('rgb(20, 22, 28)');
    const dark = await sample();

    await page.getByRole('button', { name: 'Light theme' }).click();
    await expect.poll(async () => (await sample())[0]).toBe('rgb(255, 255, 255)');
    const light = await sample();

    for (let i = 0; i < dark.length; i++) {
      expect(dark[i], `surface ${i} did not change with the theme`).not.toBe(light[i]);
    }
  });

  test('the theme survives a reload', async ({ page }) => {
    await page.getByRole('button', { name: 'Light theme' }).click();
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  });
});

// ⛔ **The request assertion that lived here now lives in `e2e/offline.spec.ts`.**
// It matched a `http://localhost:1420` prefix rather than the host, intercepted
// with `context.route` rather than observing, watched no websockets, and its
// journey never generated anything — so the one path the product exists for was
// the one path it did not cover. Two gates for one claim meant the weaker was
// the one that would rot; that file's header records what changed and why.
test.describe('Phase gate — offline and AI-free', () => {
  test('fonts are bundled, not fetched from a CDN', async ({ page }) => {
    // ⛔ **Against the page's OWN origin, not a port literal** — the last
    // survivor of the prefix match the note above retired. `localhost:1420` was
    // wrong twice over: 1420 is not free on every machine, so
    // `FMM_E2E_PORT=1431` failed this on a font it had just served correctly
    // from `http://localhost:1431/src/assets/fonts/`; and a *different* port on
    // localhost is a different server, which a `toContain` would have waved
    // through. Same origin as the document is the claim — anything else is a
    // fetch off the machine, which is what this gate exists to refuse.
    const { origin, fontUrls } = await page.evaluate(() => ({
      origin: location.origin,
      fontUrls: performance
        .getEntriesByType('resource')
        .map((e) => e.name)
        .filter((n) => /\.(woff2?|ttf|otf)(\?|$)/i.test(n)),
    }));
    for (const url of fontUrls) {
      expect(new URL(url).origin, 'fonts must be served locally').toBe(origin);
    }
  });
});

test.describe('Phase gate — resilience', () => {
  test('the app renders with no backend at all', async ({ page }) => {
    // This is exactly the case a user hits if an IPC command is missing:
    // the shell must still come up rather than showing a blank window.
    await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
    await expect(page.getByText('Search an artist. Cook.')).toBeVisible();
  });

  test('the session leaves no unhandled rejections', async ({ page }) => {
    const problems: string[] = [];
    page.on('pageerror', (e) => problems.push(e.message));
    page.on('console', (m) => {
      if (m.type() === 'error') problems.push(m.text());
    });

    await page.reload();
    await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
    for (const tab of ['Melody', 'Counter', 'Bass', 'Chords', 'Song', 'Drums']) {
      await page.getByRole('tab', { name: tab }).click();
    }
    await page.keyboard.press('k');
    await page.keyboard.press('k');

    expect(problems).toEqual([]);
  });
});
