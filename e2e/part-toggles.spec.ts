import { expect, test } from '@playwright/test';

/**
 * Switching generators on and off for playback (TASK-127).
 *
 * ⛔⛔ **Mike, 2026-08-06:** *"i want to be able to play the generators all at
 * once or separately, they should be able to be toggled on and off for each
 * generator."* Independent switches, not a solo radio: any combination is
 * legal and all-on is the default.
 *
 * ⚠ **What a browser can show, and what it cannot.** The merge itself is the
 * plugin's — `arm_pattern` takes the parts that are on and folds them into the
 * one `Pattern` a schedule can hold, and `bridge.rs`'s
 * `arming_two_generators_sounds_both_rather_than_the_last_one` is what proves
 * that. Here the claim is *reachability*: the switches exist, they appear only
 * for parts that were actually generated, and pressing one changes its state.
 */

async function generate(page: import('@playwright/test').Page, query: string) {
  const search = page.getByLabel('Search an artist');
  await search.fill(query);
  await page.getByRole('option').first().waitFor();
  await search.press('Enter');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();
}

test('no switches exist until something has been generated', async ({ page }) => {
  // ⛔ A switch for a part nobody has generated would be a control that changes
  // nothing — the shape of defect this codebase records more than any other.
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await expect(page.locator('.parttoggle')).toHaveCount(0);
});

test('a generated part gets a switch, and it starts on', async ({ page }) => {
  await page.goto('/');
  await generate(page, 'uk');

  const switches = page.locator('.parttoggle');
  await expect(switches).toHaveCount(1);
  // ⛔ ON is the default and must stay the default: a part generated after the
  // switches were last touched has to be audible, or a producer presses
  // Generate and hears nothing with no explanation on screen.
  await expect(switches.first()).toHaveAttribute('aria-pressed', 'true');
});

test('pressing a switch turns that generator off, and pressing it again turns it back on', async ({
  page,
}) => {
  await page.goto('/');
  await generate(page, 'uk');

  const drums = page.locator('.parttoggle', { hasText: 'Drums' });
  await expect(drums).toHaveAttribute('aria-pressed', 'true');

  await drums.click();
  await expect(drums).toHaveAttribute('aria-pressed', 'false');

  await drums.click();
  await expect(drums).toHaveAttribute('aria-pressed', 'true');
});

test('the switches sit beside the tabs without pushing them off the stage', async ({
  page,
}) => {
  // ⚠ The tab strip and the switches share one row now. This is the geometry
  // check that row change earned: `.tabs` gave up its border to `.stage__header`
  // so the rule runs the full width, and the switches must not overlap the tabs
  // or leave the stage.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await generate(page, 'uk');

  const header = await page.locator('.stage__header').boundingBox();
  const tabs = await page.locator('.tabs').boundingBox();
  const toggles = await page.locator('.parttoggles').boundingBox();
  expect(header && tabs && toggles).toBeTruthy();
  if (!header || !tabs || !toggles) return;

  expect(toggles.x).toBeGreaterThanOrEqual(tabs.x + tabs.width - 1);
  expect(Math.round(toggles.x + toggles.width)).toBeLessThanOrEqual(
    Math.round(header.x + header.width) + 1,
  );
});
