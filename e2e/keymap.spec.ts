import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

/**
 * The keyboard map (TASK-046, FR-018).
 *
 * `catalog.test.ts` holds the panel to the handlers — it fails when a key is
 * documented and nothing listens for it. What only a browser shows is that the
 * keys *do the thing*, and that the controls say what their key is.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await pickArtist(page, 'Mock Artist');
});

test('1 – 6 pick a generator, in the order they are drawn', async ({ page }) => {
  const tabs = page.getByRole('tab');
  const names = await tabs.allInnerTexts();

  for (const [index, name] of names.entries()) {
    await page.keyboard.press(String(index + 1));
    await expect(tabs.nth(index)).toHaveAttribute('aria-selected', 'true');
    // ⚠ And the digit is on the control, so it is learnable where the producer
    // already is rather than only in a panel they have to know exists.
    await expect(tabs.nth(index)).toHaveAttribute('title', `${name} — ${index + 1}`);
  }
});

test('G generates and Shift+G generates every part', async ({ page }) => {
  const count = page.locator('.variations__count');
  await expect(count).toHaveText('No takes yet');

  await page.keyboard.press('g');
  await expect(count).toHaveText('1 / 1');

  // ⛔ Shift+G is the only one of the three that does something different: G
  // and R are the same action, because once anything is locked "generate" *is*
  // "reroll the unlocked lanes".
  await page.keyboard.press('Shift+G');
  // Every part now has a take, which is what "all" means — the drums count
  // climbs and the melody, which had none, now has one.
  await expect(count).toHaveText('2 / 2');
  await page.getByRole('tab', { name: 'Melody' }).click();
  await expect(count).toHaveText('1 / 1');
});

test('a key pressed in a text box types rather than firing', async ({ page }) => {
  // ⛔ The rule every one of these handlers keeps: `isTypingTarget` first. A
  // producer typing "drill" into the search box must not switch tabs on the
  // "1" of a seed or generate on the "g" of "garage".
  const search = page.getByRole('combobox', { name: 'Roster' });
  await search.fill('');
  await search.press('g');
  await expect(search).toHaveValue('g');
  await expect(page.getByRole('tab').first()).toHaveAttribute('aria-selected', 'true');
});

test('the Generate buttons wear their keys', async ({ page }) => {
  await expect(page.getByRole('button', { name: 'Generate', exact: true })).toHaveAttribute(
    'title',
    /— G$/,
  );
  await expect(page.getByRole('button', { name: 'Generate all' })).toHaveAttribute(
    'title',
    /— Shift \+ G$/,
  );
});
