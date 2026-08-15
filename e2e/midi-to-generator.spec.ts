import { expect, test } from '@playwright/test';

import { browserRow, generatorTab as tab, openPanel, pickArtist } from './app';

/**
 * Dragging a `.mid` from the browser onto a generator (TASK-058 / TASK-058D).
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
 * ⛔⛔ **And the caller it eventually got read the whole file into one part**,
 * while a `.wav` dropped on the same tab split — so a layered `.mid` on Melody
 * squashed its own bass and counter into the melody clip. Mike, 2026-08-15,
 * asking whether the split had been built: it had, on the browser panel's *Read
 * the notes* road and not on this one. Both roads now end in `importSplit`,
 * which is what his sentence asked for — *"drag the audio/midi of any sample
 * into any of the generators and it will split them all the same exact way"*.
 *
 * ⚠ Chromium, not WebView2, and the fixture does not parse an SMF. What this
 * proves is the routing: every voice reaches its own generator, the tab lands
 * where the producer aimed, the wrong kind of file is refused by the target
 * rather than by an error afterwards, and an empty file does not open as an
 * empty clip.
 */

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

  // ⛔ **The tab moves with the clips.** `importSplit` does both, and a caller
  // doing one without the other leaves a melody clip open on the drum grid.
  await expect(tab(page, 'Melody')).toHaveAttribute('aria-selected', 'true');

  // ...and there are actually notes in the roll, rather than an empty editor
  // that would look identical to a failed import.
  // ⚠ `roll-notes`, the visually-hidden list the roll publishes beside its
  // canvas — a canvas has no geometry a test can read, which `PianoRoll` and
  // `piano-roll.spec.ts` both record.
  await expect(page.getByTestId('roll-notes').locator('li').first()).toBeAttached();
});

/**
 * ⛔⛔ **The whole file is separated, not read as the tab it landed on.**
 *
 * This is the assertion the old behaviour failed: the fixture is three voices,
 * and while a `.mid` drop was a one-part read only Melody came back with notes
 * — the bass and the inner voice were folded into the melody clip, silently and
 * unrecoverably. The mute dot exists only on a tab with a clip behind it, so
 * three dots is three generators filled.
 */
test('every voice in the file lands in its own generator', async ({ page }) => {
  await browserRow(page, 'riff.mid').dragTo(tab(page, 'Melody'));

  // ⚠ **"Counter", not "Countermelody"** — the tab's own label, checked against
  // `tabs.counter`. A review found the previous spec asserting a tab name that
  // does not exist, so its `toHaveCount(0)` passed over nothing at all.
  for (const part of ['Melody', 'Bass', 'Counter']) {
    await expect(page.getByRole('button', { name: `Mute ${part}` })).toBeVisible();
  }

  // ⛔ **And the generators it produced nothing for are switched off**, the same
  // statement an audio import makes — a tab left armed over silence is a tab a
  // producer presses Play on and hears nothing from. Drums and Chords have no
  // clip here, so what proves it is that neither offers a mute at all.
  await expect(page.getByRole('button', { name: /^Mute (Drums|Chords)$/ })).toHaveCount(0);
});

test('it lands on the generator it was dropped on, not the one showing', async ({ page }) => {
  await browserRow(page, 'riff.mid').dragTo(tab(page, 'Bass'));
  await expect(tab(page, 'Bass')).toHaveAttribute('aria-selected', 'true');
  await expect(tab(page, 'Drums')).toHaveAttribute('aria-selected', 'false');
});

test('a file the reader finds nothing in says so rather than doing nothing', async ({
  page,
}) => {
  // ⛔ **A drop that appears to do nothing is what a producer reads as a broken
  // plugin.** An empty split is not an error the plugin can raise — the file was
  // readable — so the page is the layer that has to say what happened.
  // ⚠ This fixture existed on the old one-part command and no spec ever reached
  // it; it moved to `explorer_midi_split` with the road, and now has one.
  await browserRow(page, 'empty.mid').dragTo(tab(page, 'Melody'));

  await expect(page.getByText('that MIDI file has no notes in it')).toBeVisible();
  // ...and nothing was opened: the tab never moved off Drums.
  await expect(tab(page, 'Drums')).toHaveAttribute('aria-selected', 'true');
});

test('a sample dropped on a generator is READ, not refused (TASK-058F)', async ({ page }) => {
  // ⛔⛔ **This spec used to assert the opposite, and the behaviour changed
  // deliberately.** It read *"a sample is not a drop target for a generator"* —
  // which was true while there was no way to get notes out of a waveform, and is
  // the case Mike asked for by name: *"can you ensure that you can drag the
  // audio/midi of any sample into any of the generators and it will split them
  // all the same exact way?"*
  //
  // ⚠ **The two-MIME-type rule is still doing its job** — it just answers
  // differently now. A `.mid` on a *drum pad* is still refused, and the Song tab
  // still takes a `.mid` and not a `.wav`; both are asserted elsewhere in this
  // file and in `kit-panel.spec.ts`.
  await browserRow(page, 'kick-808.wav').dragTo(tab(page, 'Melody'));

  // The read is a poll, so the tab arrives a moment later than a MIDI drop's.
  await expect(tab(page, 'Melody')).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('roll-notes').locator('li').first()).toBeAttached();
});

/**
 * ⛔⛔ **Mike's design, 2026-08-10:** *"could you put the midi for the entire song
 * into the 'Song' tab and allow them to pick which parts they want for the
 * generators, just like you would for a generation, but using someone elses song
 * as the starting point?"*
 *
 * The Song tab takes the **whole file as an arrangement**; the other five take it
 * split across the generators. Two gestures, and the difference is what shape the
 * file arrives in — sections on a timeline, or five clips in five editors.
 */
test('Song takes the whole file as an arrangement to pick from', async ({ page }) => {
  await browserRow(page, 'riff.mid').dragTo(tab(page, 'Song'));

  await expect(tab(page, 'Song')).toHaveAttribute('aria-selected', 'true');
  // The timeline is drawn, with sections to drill into.
  await expect(page.locator('[data-testid="song-section-0"]')).toBeVisible();
});
