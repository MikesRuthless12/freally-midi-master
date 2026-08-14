import { expect, test } from '@playwright/test';

import { browserRow, generatorTab as tab, openPanel, pickArtist } from './app';

/**
 * Reading the notes out of a sample (TASK-058D / TASK-058F / TASK-058G /
 * TASK-058H).
 *
 * Mike, 2026-08-10: *"if you drag an audio file to a generator, then it should
 * split that audio file into the correct generators, even if it's a drum
 * generation like a drum loop"*, and *"can you ensure that you can drag the
 * audio/midi of any sample into any of the generators and it will split them all
 * the same exact way?"*
 *
 * ⛔⛔ **What this proves is the WIRING, not the DSP.** The classifier is measured
 * in `plugin/src/extract/*` against synthetic fixtures and in
 * `plugin/tests/audio_extract.rs` against Mike's own files — and neither of those
 * can see whether anything on the page calls it. That gap is this repo's own
 * recorded failure: `explorer::midi_pattern` and `smf_to_pattern` shipped fully
 * tested with **no caller**, and no gate asked "is it wired up".
 *
 * ⚠ **The read is a three-command job and the mock models it as one**, which is
 * what makes the spinner and the cancel below reachable at all — see
 * `explorer_audio_status` in `src/lib/ipc-mock.ts`.
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await pickArtist(page, 'Mock Artist');
  await openPanel(page, 'explorer');
  await browserRow(page, 'Samples').click();
});

test('a sample says what is in it before anything is imported', async ({ page }) => {
  // ⛔⛔ **The rule TASK-058D states in as many words**: *"the UI names what it
  // detected … and never presents a guess as a transcription"*, so that *"a
  // wrong guess is one click to redirect rather than a silent mis-file."* A
  // button that only revealed its answer after you pressed Import would be
  // exactly the shape that forbids.
  await browserRow(page, 'kick-808.wav').click();
  await page.getByRole('button', { name: 'Read the notes' }).click();

  await expect(page.getByText('What this sample contains')).toBeVisible();
  // The parts, and the reason each was routed where it was.
  await expect(page.getByText('Struck — read from its bands')).toBeVisible();
  await expect(page.getByText('Under 250 Hz, and it holds a pitch')).toBeVisible();
  // ⚠ The weakest reason says so: leads, keys, guitars, brass and voices all
  // share that band.
  await expect(page.getByText('The clearest line up top — a guess')).toBeVisible();

  // Nothing has been imported yet — the generators are untouched.
  await expect(tab(page, 'Drums')).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('roll-notes').locator('li').first()).not.toBeAttached();
});

test('sending it to the generators opens every part it found', async ({ page }) => {
  // ⛔⛔ **A part is generated FIRST, on purpose.** TASK-058H's switch-off is
  // only visible on a tab that has a clip behind it — the dot does not exist
  // otherwise — and the case that actually matters is exactly this one: a part
  // the producer made earlier, still sounding under a song that does not contain
  // it. Without the generation this test could not fail.
  await tab(page, 'Chords').click();
  // ⚠ `exact`, because "Generate all" is beside it — the same guard
  // `arrangement.spec.ts` uses.
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Mute Chords' })).toBeVisible();

  await openPanel(page, 'explorer');
  await browserRow(page, 'kick-808.wav').click();
  await page.getByRole('button', { name: 'Read the notes' }).click();
  await page.getByRole('button', { name: 'Send to generators' }).click();

  // The parts it found are open and sounding. Drums is the biggest, so that is
  // where the tab lands.
  await expect(tab(page, 'Drums')).toHaveAttribute('aria-selected', 'true');
  for (const part of ['Drums', 'Bass', 'Melody']) {
    await expect(page.getByRole('button', { name: `Mute ${part}` })).toBeVisible();
  }

  // ⛔⛔ **And Chords, which the split did not produce, is switched off** — the
  // dot now offers to *un*mute it. Mike: *"if there is no countermelody or no
  // bassline that it mutes them."*
  // ⚠ Switched off, never deleted: the clip is still there, which is what makes
  // the dot the way back.
  await expect(page.getByRole('button', { name: 'Unmute Chords' })).toBeVisible();
});

test('a sung line is left alone, and the panel says so', async ({ page }) => {
  // ⛔⛔ **Mike: *"and leave the vocals alone."*** The outcome a producer is most
  // likely to read as a broken plugin is the one where nothing arrives, so it
  // has to be *named*.
  await browserRow(page, 'hook-vocal.wav').click();
  await page.getByRole('button', { name: 'Read the notes' }).click();

  await expect(page.getByText('A sung line was found, and left alone.')).toBeVisible();
  // ...and there is nothing to send, so the button is not offered.
  await expect(page.getByRole('button', { name: 'Send to generators' })).toHaveCount(0);
});

test('a read in flight can be stopped', async ({ page }) => {
  // ⚠ Reachable because reading a real file takes seconds — `extract::job` is
  // honest about what stopping does: it releases the producer, not the CPU.
  await browserRow(page, 'clap-01.wav').click();
  await page.getByRole('button', { name: 'Read the notes' }).click();
  await page.getByRole('button', { name: 'Stop reading' }).click();

  // Back to the offer, with nothing imported.
  await expect(page.getByRole('button', { name: 'Read the notes' })).toBeVisible();
  await expect(page.getByText('What this sample contains')).toHaveCount(0);
});

test('walking to another file clears the answer about the last one', async ({ page }) => {
  // ⛔ The readout-that-lies failure in miniature: a panel that went on showing
  // what the *previous* sample contained, under this file's name.
  await browserRow(page, 'kick-808.wav').click();
  await page.getByRole('button', { name: 'Read the notes' }).click();
  await expect(page.getByText('What this sample contains')).toBeVisible();

  await browserRow(page, 'clap-01.wav').click();
  await expect(page.getByText('What this sample contains')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Read the notes' })).toBeVisible();
});

test('a .mid is not offered the sample reader', async ({ page }) => {
  // ⚠ TASK-058's two-kinds rule, at the one panel that could forget it: a `.mid`
  // already *has* its notes, and `MidiPreview` shows them on selection.
  await browserRow(page, 'riff.mid').click();
  await expect(page.getByText('What this MIDI file contains')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Read the notes' })).toHaveCount(0);
});
