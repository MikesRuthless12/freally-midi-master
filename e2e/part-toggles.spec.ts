import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

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
  // ⚠ The shared helper, not a local copy of the gesture. This waited for an
  // option to become *visible* before pressing Enter, which the portalled menu
  // does not satisfy in the same frame — and `pickArtist` also blurs, which the
  // app requires before any keyboard shortcut will fire.
  await pickArtist(page, query);
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();
}

test('no switches exist until something has been generated', async ({ page }) => {
  // ⛔ A switch for a part nobody has generated would be a control that changes
  // nothing — the shape of defect this codebase records more than any other.
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await expect(page.locator('.tab-mute')).toHaveCount(0);
});

test('a generated part gets a switch, and it starts on', async ({ page }) => {
  await page.goto('/');
  await generate(page, 'uk');

  const switches = page.locator('.tab-mute');
  await expect(switches).toHaveCount(1);
  // ⛔ ON is the default and must stay the default: a part generated after the
  // switches were last touched has to be audible, or a producer presses
  // Generate and hears nothing with no explanation on screen.
  await expect(switches.first()).toHaveAttribute('data-on', 'true');
});

test('pressing a switch turns that generator off, and pressing it again turns it back on', async ({
  page,
}) => {
  await page.goto('/');
  await generate(page, 'uk');

  const drums = page.locator('.tab-slot', { hasText: 'Drums' }).locator('.tab-mute');
  await expect(drums).toHaveAttribute('data-on', 'true');

  await drums.click();
  await expect(drums).toHaveAttribute('data-on', 'false');

  await drums.click();
  await expect(drums).toHaveAttribute('data-on', 'true');
});

test('the switch sits inside its own tab, at the top right', async ({ page }) => {
  // ⛔⛔ **The geometry Mike asked for by name**, 2026-08-09: *"it should be a
  // button … in the top right side of the tab, so that way you can mute it by
  // just clicking the 'Green' dot."* The switches used to be a separate row
  // beside the tab strip carrying the same six words, so working out which one
  // silenced Drums meant reading "Drums" twice and matching them up.
  //
  // ⚠ Pinned as position rather than existence, for the reason the row version
  // gave: the control was always there, so nothing would catch it drifting back
  // out of the tab.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await generate(page, 'uk');

  const slot = page.locator('.tab-slot', { hasText: 'Drums' });
  const tab = await slot.locator('.tab').boundingBox();
  const dot = await slot.locator('.tab-mute').boundingBox();
  expect(tab && dot).toBeTruthy();
  if (!tab || !dot) return;

  // Inside the tab it belongs to, vertically and horizontally.
  expect(dot.x).toBeGreaterThanOrEqual(tab.x);
  expect(dot.x + dot.width).toBeLessThanOrEqual(tab.x + tab.width + 1);
  expect(dot.y).toBeGreaterThanOrEqual(tab.y);

  // In its top *right* quarter — the half of the instruction a "is it in the
  // tab" check would miss.
  expect(dot.x).toBeGreaterThan(tab.x + tab.width / 2);
  expect(dot.y).toBeLessThan(tab.y + tab.height / 2);
});
