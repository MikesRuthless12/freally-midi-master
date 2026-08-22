/**
 * Where the marks sit in the picture that rides on the cursor (TASK-063C).
 *
 * Mike, 2026-08-05: *"ensure it shows a preview of what you are dragging"*.
 *
 * ⛔ **Separate from the drawing, because only this half can be tested.** A
 * canvas needs a real 2D context and the pixels end up inside a Windows DIB;
 * what can be checked here is the part with a decision in it — which notes land
 * where, what happens to a clip with one pitch in it, and what happens to a clip
 * with none. `SongTimeline/geometry.ts` is split from `sketch.ts` for the same
 * reason. ⚠ Everything the drag image draws goes through here, including the
 * arrangement's bars: the first cut computed those inline in the canvas file,
 * which left the one shape with no test in the file whose header says why that
 * must not happen.
 */

import { patternTicks } from '../PianoRoll/notes';
import { partsInUse, totalBars } from '../SongTimeline/clips';
import type { Lane, Note, Pattern, Song } from '../../lib/ipc-types';

/** The drag image, in pixels. Big enough to read, small enough not to cover the drop target. */
export const PREVIEW_WIDTH = 260;
export const PREVIEW_HEIGHT = 92;

/** The strip at the top that holds the stem's name. */
export const PREVIEW_LABEL_HEIGHT = 26;

/** The thinnest a note may draw. A one-tick note still has to be visible. */
export const MIN_NOTE_WIDTH = 3;

/** The most rows the notes are spread over, however many pitches there are. */
export const MAX_ROWS = 14;

export type PreviewBar = {
  x: number;
  y: number;
  width: number;
  height: number;
  /** From the note's velocity, so a ghost note reads as one. */
  alpha: number;
};

/**
 * How long the pattern is, in ticks.
 *
 * ⛔ **`patternTicks`, not a private copy.** `SongTimeline/sketch.ts` records
 * that it "is the one definition of that in the app; a private copy here would
 * have been the ninth", and `DrumGrid/cells.ts` carries the note about the bug
 * one caused. This is a re-export so the drag image cannot be the tenth.
 */
export { patternTicks };

/**
 * Every note as a rectangle inside the picture.
 *
 * Pitch is mapped across whatever range the clip actually uses rather than
 * across all 128 — a four-note bassline drawn on a full keyboard is four marks
 * in a smear of empty space, which tells the producer nothing about which loop
 * they picked up.
 */
export function previewBars(notes: Note[], ticks: number): PreviewBar[] {
  if (notes.length === 0 || ticks <= 0) return [];

  let low = notes[0].pitch;
  let high = notes[0].pitch;
  for (const note of notes) {
    if (note.pitch < low) low = note.pitch;
    if (note.pitch > high) high = note.pitch;
  }
  // ⚠ A clip on one pitch — a kick lane, most of the time — has a span of zero,
  // and dividing by it would put every note at `NaN`. One row is also the
  // honest drawing of it: `rows` falls to 1, so every note lands on row 0.
  const span = high - low;
  const rows = Math.min(MAX_ROWS, span + 1);
  const rowHeight = (PREVIEW_HEIGHT - PREVIEW_LABEL_HEIGHT) / rows;

  return notes.map((note) => {
    const fraction = span === 0 ? 0 : (note.pitch - low) / span;
    // High notes at the top, which is the way both editors in this app draw.
    const row = Math.round((1 - fraction) * (rows - 1));
    return {
      x: (note.startTick / ticks) * PREVIEW_WIDTH,
      y: PREVIEW_LABEL_HEIGHT + row * rowHeight,
      width: Math.max(MIN_NOTE_WIDTH, (note.lenTicks / ticks) * PREVIEW_WIDTH),
      height: Math.max(2, rowHeight - 1),
      // Floored well above zero: a note drawn at velocity 1 is invisible, and
      // "the clip is empty" is the one thing this picture must never say by
      // accident.
      alpha: 0.45 + 0.55 * Math.min(1, note.vel / 127),
    };
  });
}

/**
 * Every note the drop will actually contain.
 *
 * ⛔ **`lane` is not optional decoration.** A per-lane drag sends the pattern
 * whole and names the lane, leaving the cut to `export::Cut` — so a picture
 * drawn from every lane shows the producer the whole kit while the file they
 * are carrying holds one lane, and all eight lane chips draw the same image.
 */
export function allNotes(patterns: Pattern[], lane?: Lane): Note[] {
  return patterns.flatMap((pattern) =>
    pattern.lanes
      .filter((track) => lane === undefined || track.lane === lane)
      .flatMap((track) => track.notes),
  );
}

/**
 * One clip of an arrangement, where it will land.
 *
 * ⚠ Carries its row rather than a part name: the picture is 260 pixels wide and
 * has no room for labels, so what a producer reads is the *shape* — five rows
 * of blocks stepping along a timeline is their record, and they recognise it.
 */
export type PreviewClip = Omit<PreviewBar, 'alpha'>;

/** The gap between rows, so five parts do not read as one block. */
const CLIP_GAP = 1;

/** The thinnest a clip may draw, so a one-bar intro is still a rectangle. */
export const MIN_CLIP_WIDTH = 2;

/**
 * The arrangement as it will land in the DAW (TASK-144).
 *
 * ⛔⛔ **Mike, 2026-08-06, correcting his own first request:** *"i don't want it
 * to show how many midi/audio files, i want the song arrangement being dragged
 * in to actually show the midi clips either together back to back or stacked
 * with the 'Ctrl' or 'Command' keys being pressed, same with audio clips."* So
 * this is a miniature of the record, not a legend and not a count — clip
 * rectangles at their true bar positions and true lengths, one row per part,
 * the way `SongTimeline` already draws them.
 *
 * ▶ **`stacked` is the modifier's layout, and both are knowable before the drag
 * begins** — the plugin spools `Prepared::paths` and `Prepared::stacked` up
 * front, so neither needs anything rendered to draw it.
 * - **Back to back** (no modifier): every clip at its own bar, which is the
 *   arrangement.
 * - **Stacked** (Ctrl / Command): the same clips overlaid at bar 1, one row per
 *   part. They overlap on purpose — that is what stacking *does*, and a picture
 *   that tidied them into a row would be describing a third layout that no
 *   modifier produces.
 *
 * ⛔ **The bitmap is fixed when the drag starts and cannot be redrawn as Ctrl
 * goes down** — `drag/windows.rs` hands it to `IDragSourceHelper` ahead of
 * `DoDragDrop`. So this draws the layout that was true at that moment, which is
 * the honest limit rather than a bug. The *payload* still swaps mid-gesture,
 * which already works.
 *
 * ⚠ **Empty is a real answer**, and the caller must treat it as one: a song with
 * no sections, or one whose sections carry no patterns, draws nothing rather
 * than a full-width block claiming a record that is not there.
 */
export function songClips(song: Song, stacked = false): PreviewClip[] {
  // ⛔ **The rows are the parts the song actually plays**, not all five. A row
  // for a part nothing plays is a claim that the record has a countermelody in
  // it. `partsInUse` is the one definition of that, in `SongTimeline/clips.ts`.
  const rows = partsInUse(song);
  if (rows.length === 0) return [];

  // ⛔ **`totalBars`, not a private maximum.** This file's own header states the
  // rule — *"`patternTicks`, not a private copy … a private copy here would have
  // been the ninth"* — and `SongTimeline` scales its width from exactly this, so
  // a second definition is how the cursor and the timeline come to draw
  // different pictures of one arrangement.
  const total = totalBars(song);
  if (total <= 0) return [];

  const rowHeight = (PREVIEW_HEIGHT - PREVIEW_LABEL_HEIGHT) / rows.length;

  return song.sections.flatMap((section) =>
    rows.flatMap((part, row) =>
      section.patterns[part]
        ? [
            {
              // ⚠ Stacked puts every clip at bar 1 and keeps its own length —
              // the two halves of what the producer sees when they drop with
              // Ctrl held.
              x: stacked ? 0 : (section.startBar / total) * PREVIEW_WIDTH,
              y: PREVIEW_LABEL_HEIGHT + row * rowHeight,
              width: Math.max(
                MIN_CLIP_WIDTH,
                (section.bars / total) * PREVIEW_WIDTH - CLIP_GAP,
              ),
              height: Math.max(2, rowHeight - CLIP_GAP),
            },
          ]
        : [],
    ),
  );
}
