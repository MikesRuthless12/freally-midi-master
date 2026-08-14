import { expect, test, type Page } from '@playwright/test';

import { browserRow, openPanel } from './app';

/**
 * The pad's own sound, end to end (TASK-055A, TASK-164, TASK-059).
 *
 * ⛔⛔ **The gap this closes is the one Mike opened the whole task with:** he
 * asked whether the drum lanes had an editor — *"Kick, Sub Bass, Rim Shot,
 * etc."* — and the answer was that the **notes** had one and the **sound** did
 * not. `plugin/src/pad_tweaks.rs` and `audio/sampler.rs` cover the arithmetic
 * and `state/kit.test.ts` covers the store; what only a browser shows is that a
 * control a producer can actually reach is wired to them.
 *
 * ⚠ **The mock had to store for any of this to be assertable.** `pad_tweaks_set`
 * answering `undefined` and `kit_state` answering constants would let a spec
 * turn every knob and check nothing — the pad would read the same before and
 * after. That is the recorded reason the browser→pad gesture went untested for
 * months, and it applies to a knob more than to a label. See `padTweaks` and
 * `previewLevel` in `ipc-mock.ts`.
 *
 * ⚠ Chromium, not WebView2, and no audio thread. What this proves is the page's
 * half: the control exists, it sends the whole block, and what comes back on the
 * next read is what was sent. That the *sound* changes is `plugin/`'s tests and,
 * in the end, `Live-To-Do.md`'s human in a DAW.
 */

/** The pad holding `lane`, by its own data attribute rather than by text. */
function pad(page: Page, lane: string) {
  return page.locator(`.pad[data-lane="${lane}"]`);
}

/** The KIT row for `lane`. */
function row(page: Page, lane: string) {
  return page.locator(`.kit-lane[data-lane="${lane}"]`);
}

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});

test('a shipped lane can be shaped, not only one carrying your own sample', async ({
  page,
}) => {
  // ⛔ The whole point of TASK-164. An editor that only opened over a replaced
  // sample would leave Mike's question unanswered for every pad nobody had
  // swapped, which is most of them.
  await openPanel(page, 'kit');
  const melody = row(page, 'melody');
  await expect(melody).toHaveAttribute('data-assigned', 'false');

  await melody.getByRole('button', { name: /^Edit the sound of / }).click();
  await expect(page.getByRole('dialog', { name: /Melody — sound/ })).toBeVisible();
});

test('a lane with no voice at all is offered no editor', async ({ page }) => {
  // ⚠ The same `canSound` predicate the Play button is behind. An envelope over
  // a lane with no voice is a control that can only do nothing.
  await openPanel(page, 'kit');
  await expect(
    row(page, 'snap').getByRole('button', { name: /^Edit the sound of / }),
  ).toHaveCount(0);
});

test('a control keeps its value when the panel is read again', async ({ page }) => {
  // ⛔ **This is the assertion the mock exists for.** Reading the value back off
  // a *fresh* `kit_state` is what distinguishes a knob wired to the plugin from
  // a knob wired to a `useState` — the second passes every check made without
  // leaving the panel.
  await openPanel(page, 'kit');
  await row(page, 'kick')
    .getByRole('button', { name: /^Edit the sound of / })
    .click();

  const volume = page.getByRole('dialog').getByLabel('Volume');
  await volume.fill('-6');
  await expect(volume).toHaveValue('-6');

  // Away and back: the panel unmounts and re-reads `kit_state` on remount.
  //
  // ⚠ **No second click.** Which pad is being edited lives in the store, so it
  // survives the panel going away — coming back puts the producer where they
  // were, and pressing the button again would *close* it. That is the behaviour;
  // this spec found it by asserting the opposite.
  await openPanel(page, 'genres');
  await openPanel(page, 'kit');
  await expect(page.getByRole('dialog').getByLabel('Volume')).toHaveValue('-6');
});

test('Reset returns every control at once', async ({ page }) => {
  // ⚠ One command, not ten — field by field would rebuild the kit ten times and
  // let the audio thread hear nine states that were never on screen. What a
  // browser can check is that all of them came back.
  await openPanel(page, 'kit');
  await row(page, 'kick')
    .getByRole('button', { name: /^Edit the sound of / })
    .click();

  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Volume').fill('-6');
  await dialog.getByLabel('Transpose').fill('7');
  await dialog.getByRole('button', { name: 'Reset' }).click();

  await expect(dialog.getByLabel('Volume')).toHaveValue('0');
  await expect(dialog.getByLabel('Transpose')).toHaveValue('0');
});

test('the trim window closes rather than inverting when the handles cross', async ({
  page,
}) => {
  // ⛔ The plugin clamps an inverted slice — it is the shape that once aborted
  // the host — but a slider that lets you drag past and silently snaps back is a
  // control fighting its own user, and only a browser shows which of the two
  // this is.
  await openPanel(page, 'kit');
  await row(page, 'kick')
    .getByRole('button', { name: /^Edit the sound of / })
    .click();

  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('End').fill('0.3');
  await dialog.getByLabel('Start').fill('0.8');

  await expect(dialog.getByLabel('Start')).toHaveValue('0.8');
  await expect(dialog.getByLabel('End')).toHaveValue('0.8');
});

test('dropping a sample on a pad opens that pad’s editor', async ({ page }) => {
  // ⛔⛔ **TASK-059's missing half.** The two assignment routes shipped with
  // TASK-059A; what the entry actually asks for is "imports, assigns, **and
  // opens the per-one-shot editor**". The pad is on the stage and the editor is
  // drawn in the rail, so this also proves the KIT panel is brought on screen —
  // an editor that opened where nobody could see it would pass a store test and
  // fail a producer.
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();

  await browserRow(page, 'kick-hard.wav').dragTo(pad(page, 'snare'));

  await expect(pad(page, 'snare')).toHaveAttribute('data-assigned', 'true');
  await expect(page.getByRole('dialog', { name: /Snare — sound/ })).toBeVisible();
});

test('the browser can play a file at its own level, or exactly as it is', async ({ page }) => {
  // ⛔ TASK-058B's last piece. The audition has always sat below full scale so it
  // does not clip over a running pattern; `Raw` is what makes one of the two
  // things a producer is comparing be the file.
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
  await browserRow(page, 'Kicks').click();
  await browserRow(page, 'kick-hard.wav').click();

  const raw = page.getByRole('button', { name: 'Raw' });
  await expect(raw).toHaveAttribute('aria-pressed', 'false');

  const level = page.getByLabel('Audition level');
  await level.fill('-6');
  await raw.click();
  await expect(raw).toHaveAttribute('aria-pressed', 'true');

  // ⛔ **The level survives the bypass**, which is the difference between two
  // controls and one: a producer who A/Bs against the file has to come back to
  // what they had dialled in.
  await expect(level).toHaveValue('-6');
  await raw.click();
  await expect(raw).toHaveAttribute('aria-pressed', 'false');
  await expect(level).toHaveValue('-6');
});
