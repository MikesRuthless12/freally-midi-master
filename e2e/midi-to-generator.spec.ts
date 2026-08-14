import { expect, test, type Page } from '@playwright/test';

import { browserRow, openPanel, pickArtist } from './app';

/**
 * Dragging a `.mid` from the browser onto a generator (TASK-058 / TASK-040T).
 *
 * Mike, 2026-08-10: *"i want to be able to view .mid files in the File Explorer
 * … and be able to drag the file to a generator."*
 *
 * ⛔⛔ **The Rust for this shipped with no caller**, exactly as the whole
 * explorer did before TASK-132: `explorer::midi_pattern` and
 * `engine::smf_read::smf_to_pattern` were written, tested from Rust, and nothing
 * in `src/` invoked either. The handoff recorded it — *"`explorer_midi` answers
 * and nothing calls it"* — and it stayed that way because no gate in the repo
 * asks "is it wired up".
 *
 * ⚠ Chromium, not WebView2, and the fixture does not parse an SMF. What this
 * proves is the routing: the drop reaches the part it was dropped on, the wrong
 * kind of file is refused by the target rather than by an error afterwards, and
 * an empty file does not open as an empty clip.
 */

function tab(page: Page, name: string) {
  return page.getByRole('tablist', { name: 'Generator' }).getByRole('tab', { name });
}

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await pickArtist(page, 'Mock Artist');
  // ⛔ The panel is behind a vertical tab now — `openPanel` presses it.
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
});

test('a .mid dropped on a generator lands there as notes', async ({ page }) => {
  // Melody rather than Drums: the default tab is Drums, so dropping there could
  // pass while the tab never moved.
  await browserRow(page, 'riff.mid').dragTo(tab(page, 'Melody'));

  // ⛔ **The tab moves with the clip.** `openClip` does both, and a caller doing
  // one without the other leaves a melody clip open on the drum grid.
  await expect(tab(page, 'Melody')).toHaveAttribute('aria-selected', 'true');

  // ...and there are actually notes in the roll, rather than an empty editor
  // that would look identical to a failed import.
  // ⚠ `roll-notes`, the visually-hidden list the roll publishes beside its
  // canvas — a canvas has no geometry a test can read, which `PianoRoll` and
  // `piano-roll.spec.ts` both record.
  await expect(page.getByTestId('roll-notes').locator('li').first()).toBeAttached();
});

test('it lands on the generator it was dropped on, not the one showing', async ({ page }) => {
  await browserRow(page, 'riff.mid').dragTo(tab(page, 'Bass'));
  await expect(tab(page, 'Bass')).toHaveAttribute('aria-selected', 'true');
  await expect(tab(page, 'Drums')).toHaveAttribute('aria-selected', 'false');
});

test('a sample is not a drop target for a generator', async ({ page }) => {
  // ⛔ The other half of the two-MIME-type rule. A `.wav` on a generator and a
  // `.mid` on a drum pad are both controls that can only fail, so each target
  // refuses the other *before* the drop rather than erroring after it.
  await browserRow(page, 'kick-808.wav').dragTo(tab(page, 'Melody'));

  // Nothing opened: Drums is still the tab, and the melody has no clip.
  await expect(tab(page, 'Drums')).toHaveAttribute('aria-selected', 'true');
});

/**
 * ⛔⛔ **Mike's design, 2026-08-10:** *"could you put the midi for the entire song
 * into the 'Song' tab and allow them to pick which parts they want for the
 * generators, just like you would for a generation, but using someone elses song
 * as the starting point?"*
 *
 * The Song tab takes the **whole file as an arrangement**; the other five take it
 * into that one generator. Two gestures, and the difference is which question the
 * producer is asking — "I know what this is" versus "show me what is in it".
 */
test('Song takes the whole file as an arrangement to pick from', async ({ page }) => {
  await browserRow(page, 'riff.mid').dragTo(tab(page, 'Song'));

  await expect(tab(page, 'Song')).toHaveAttribute('aria-selected', 'true');
  // The timeline is drawn, with sections to drill into.
  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();
});
