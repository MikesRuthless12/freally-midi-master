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
import type { Lane, Note, Pattern } from '../../lib/ipc-types';

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
 * The arrangement's shape, one bar per bucket.
 *
 * ⚠ **The sections, not the notes.** Drawing every note of a three-minute record
 * into 260 pixels is a solid block, which says nothing; the shape of the
 * arrangement is what a producer recognises. `levels` is `sectionDensity`'s
 * reading — the same one `SongTimeline` draws its sketch from.
 */
export function sectionBars(levels: number[]): PreviewBar[] {
  if (levels.length === 0) return [];
  const width = PREVIEW_WIDTH / levels.length;
  const tallest = PREVIEW_HEIGHT - PREVIEW_LABEL_HEIGHT - 8;
  return levels.map((level, index) => {
    const height = Math.max(2, Math.min(1, Math.max(0, level)) * tallest);
    return {
      x: index * width + 1,
      y: PREVIEW_HEIGHT - 4 - height,
      width: Math.max(1, width - 2),
      height,
      alpha: 0.4 + 0.6 * Math.min(1, Math.max(0, level)),
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
