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
    await expect(page.getByLabel('Search an artist')).toBeVisible();
    await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Generate' })).toBeVisible();
    await expect(page.getByRole('button', { name: /Kit/i })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Play' })).toBeVisible();
  });

  test('the empty state uses the product voice', async ({ page }) => {
    // The vision is explicit about this copy; a generic "No data" would be a
    // brand regression, not a cosmetic one.
    await expect(page.getByText('Search an artist. Cook.')).toBeVisible();
  });

  test('nothing claims to work before it does', async ({ page }) => {
    for (const name of ['Generate', 'Play', 'Stop', 'Loop']) {
      await expect(page.getByRole('button', { name })).toBeDisabled();
    }
    await expect(page.getByLabel('Search an artist')).toBeDisabled();
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

  test('collapsible panels expose aria-expanded', async ({ page }) => {
    for (const name of [/Genres/i, /Roster/i, /Kit/i, /Session/i]) {
      await expect(page.getByRole('button', { name })).toHaveAttribute(
        'aria-expanded',
        /true|false/,
      );
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

test.describe('Phase gate — offline and AI-free', () => {
  test('the UI makes no network requests of its own', async ({ page, context }) => {
    // Generation, playback and export never touch the network. The updater is
    // native, so nothing should leave the page at all.
    const external: string[] = [];
    await context.route('**/*', (route) => {
      const url = route.request().url();
      if (!url.startsWith('http://localhost:1420') && !url.startsWith('data:')) {
        external.push(url);
      }
      return route.continue();
    });

    await page.reload();
    await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
    await page.getByRole('tab', { name: 'Chords' }).click();

    expect(external, `the UI reached out to: ${external.join(', ')}`).toEqual([]);
  });

  test('fonts are bundled, not fetched from a CDN', async ({ page }) => {
    const fontUrls = await page.evaluate(() =>
      performance
        .getEntriesByType('resource')
        .map((e) => e.name)
        .filter((n) => /\.(woff2?|ttf|otf)(\?|$)/i.test(n)),
    );
    for (const url of fontUrls) {
      expect(url, 'fonts must be served locally').toContain('localhost:1420');
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
