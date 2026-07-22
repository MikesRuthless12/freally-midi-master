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
  await expect(page.getByLabel('Search an artist')).toBeVisible();
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Play' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Generate' })).toBeVisible();
  await expect(page.getByText('Search an artist. Cook.')).toBeVisible();
});

test('all six generator tabs are present', async ({ page }) => {
  const tabs = page.getByRole('tab');
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
  await expect(page.getByLabel('Search an artist')).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Generate' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Play' })).toBeDisabled();
});

test('K toggles the right rail', async ({ page }) => {
  const kit = page.getByRole('button', { name: /Kit/i });
  await expect(kit).toBeVisible();

  await page.keyboard.press('k');
  await expect(kit).toBeHidden();

  await page.keyboard.press('k');
  await expect(kit).toBeVisible();
});

test('a panel collapses from its header and stays collapsed across a reload', async ({
  page,
}) => {
  const genres = page.getByRole('button', { name: /Genres/i });
  await expect(genres).toHaveAttribute('aria-expanded', 'true');

  await genres.click();
  await expect(genres).toHaveAttribute('aria-expanded', 'false');

  await page.reload();
  await expect(page.getByRole('button', { name: /Genres/i })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
});

test('the View menu lists every panel', async ({ page }) => {
  await page.getByRole('button', { name: /View/i }).click();
  const items = page.getByRole('menuitemcheckbox');
  await expect(items).toHaveCount(5);
  await expect(items.first()).toContainText('Right rail');
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
