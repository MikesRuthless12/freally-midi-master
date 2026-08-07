import { expect, test } from '@playwright/test';
import { ALL_LANES } from '../src/state/lanes';

/**
 * The KIT panel, end to end (TASK-131B, TASK-136).
 *
 * ⛔ **This is the gate for a defect a human found and no gate could.** Mike
 * loaded the plugin in Ableton on 2026-08-04 and the KIT panel read "No kit yet"
 * while a twelve-pad kit was loaded and audibly playing. It said that because it
 * was eight hardcoded `disabled` buttons and a static string, connected to
 * nothing at all — so there was no assertion anywhere that could have failed.
 *
 * `src/state/kit.test.ts` covers the store's rules on their own. What only a
 * browser shows is that the panel is actually wired to them: that it draws a row
 * per lane from what the plugin answered, and that a lane with no voice reads
 * differently from one that has one.
 *
 * ⚠ The mock reports a *cancelled* assignment, because a browser has no native
 * Open dialog and no filesystem — so what can be checked here is the panel's
 * shape and its refusal to strand itself, not a sample actually loading. That
 * half is `Live-To-Do.md` § 4.9, and it needs a human in a DAW.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('the panel draws a row per lane instead of eight numbered placeholders', async ({
  page,
}) => {
  const lanes = page.locator('.kit-lane');
  // ⚠ **Derived, never a number typed here.** This read 13 and TASK-140 took
  // the engine to 21 lanes — a hardcoded count in a test is the same failure
  // the panel itself had before TASK-136, where a list written in the UI
  // stopped matching the kit that was actually loaded.
  await expect(lanes).toHaveCount(ALL_LANES.length);

  // Named, not numbered. This is the whole difference.
  await expect(page.locator('.kit-lane[data-lane="kick"]')).toContainText('Kick');
  await expect(page.locator('.kit-lane[data-lane="melody"]')).toContainText('Melody');
  await expect(page.locator('.kit-lane[data-lane="chords"]')).toContainText('Chords');

  // And nothing on screen claims there is no kit.
  await expect(page.getByText('No kit yet')).toHaveCount(0);
});

test('a lane with a voice reads differently from one with none', async ({ page }) => {
  // ⚠ `snap` is in the drum generator's lane list and the shipped kit has never
  // carried a pad for it, so it renders silence. The panel has to be able to say
  // that — it is the one lane assigning a one-shot is the *only* way to hear.
  await expect(page.locator('.kit-lane[data-lane="snap"]')).toHaveAttribute(
    'data-silent',
    'true',
  );
  await expect(page.locator('.kit-lane[data-lane="kick"]')).toHaveAttribute(
    'data-silent',
    'false',
  );

  // Nothing is the producer's own yet, so no row offers to clear one.
  await expect(page.locator('.kit-lane__clear')).toHaveCount(0);
});

test('closing the dialog leaves the panel usable rather than stuck on "Choosing…"', async ({
  page,
}) => {
  // ⛔ The failure this guards is one the export shipped once: a poll that
  // mishandles its terminal status leaves the control disabled for the rest of
  // the session, with nothing on screen to explain it. The mock answers
  // `cancelled`, which is the ordinary way out of an Open dialog.
  const melody = page.locator('.kit-lane[data-lane="melody"] .kit-lane__pad');
  await melody.click();

  await expect(melody).toBeEnabled();
  // ...and it is not showing an error either, because closing a dialog is not
  // a failure.
  await expect(page.locator('.kit-error')).toHaveCount(0);
  await expect(melody).toContainText('Built in');
});
