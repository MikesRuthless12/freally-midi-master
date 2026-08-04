/**
 * Where a tick and a pitch land on screen, and back again (TASK-041).
 *
 * Pure, and separate from both the note data and the canvas: the renderer and
 * the pointer handlers must agree exactly, and the only way to guarantee that is
 * for them to call the same functions. A hit test that rounds differently from
 * the draw is a note you can see but cannot click, which is the classic canvas
 * editor bug and is invisible to any test that only checks the handler fired.
 */

/** What the roll is looking at. Everything below is derived from this. */
export type View = {
  /** Pixels per quarter note. */
  zoom: number;
  /** Pixels per semitone row. */
  rowHeight: number;
  /** Leftmost visible tick. */
  scrollTick: number;
  /** Pitch drawn on the top row. Rows count *down* from here. */
  topPitch: number;
  /** Ticks per quarter note — the engine's resolution, carried on the pattern. */
  ppq: number;
};

/** Horizontal: a tick's pixel offset from the left edge of the canvas. */
export function tickToX(tick: number, view: View): number {
  return ((tick - view.scrollTick) / view.ppq) * view.zoom;
}

/** Horizontal: which tick a pixel offset falls on. */
export function xToTick(x: number, view: View): number {
  return (x / view.zoom) * view.ppq + view.scrollTick;
}

/**
 * The pitches the roll is currently drawing, top row first.
 *
 * ⛔ **A list rather than a range, because folding removes rows from the
 * middle** (TASK-041B). Once `Fold to scale` or Ableton's note `Fold` is on,
 * the visible rows are no longer contiguous, so "which pitch is this row" stops
 * being subtraction. Every vertical conversion goes through this list, which is
 * what keeps the renderer and the hit test agreeing about a folded roll — the
 * failure otherwise is notes you can see and cannot click, and it only appears
 * once folding is switched on.
 *
 * ⛔ Pitch increases *upward*. A roll with low notes at the top is unreadable to
 * anyone who has used one before.
 */
export type Rows = {
  /** Visible pitches, highest first. */
  pitches: number[];
  /** Where each pitch sits in `pitches`, for the reverse lookup. */
  indexOf: Map<number, number>;
};

/**
 * Build the visible row list.
 *
 * `keep` decides what is in it — the folds pass their predicate in rather than
 * this knowing what a scale is. `height` bounds the work: there is no point
 * walking to pitch 0 for a window that shows twenty rows.
 */
export function rowsFor(view: View, height: number, keep: (pitch: number) => boolean): Rows {
  const wanted = Math.ceil(height / view.rowHeight) + 1;
  const pitches: number[] = [];
  const indexOf = new Map<number, number>();
  for (
    let pitch = Math.min(127, view.topPitch);
    pitch >= 0 && pitches.length < wanted;
    pitch -= 1
  ) {
    if (!keep(pitch)) continue;
    indexOf.set(pitch, pitches.length);
    pitches.push(pitch);
  }
  return { pitches, indexOf };
}

/** Vertical: a row index's pixel offset from the top of the canvas. */
export function rowY(index: number, view: View): number {
  return index * view.rowHeight;
}

/** Vertical: a pitch's pixel offset, or `null` when it is folded away. */
export function pitchToY(pitch: number, rows: Rows, view: View): number | null {
  const index = rows.indexOf.get(pitch);
  return index === undefined ? null : rowY(index, view);
}

/** Vertical: which pitch a pixel offset falls on, or `null` past the last row. */
export function yToPitch(y: number, rows: Rows, view: View): number | null {
  const index = Math.floor(y / view.rowHeight);
  return rows.pitches[index] ?? null;
}

/**
 * How wide a note of `lenTicks` is drawn.
 *
 * ⛔ **Floored at two pixels, and the hit test uses the same floor.** The draw
 * floored it and `noteAt` did not, so at low zoom a 1/64 was visible over two
 * pixels and clickable over a fraction of one — the see-it-cannot-click-it case
 * this module exists to prevent. One function, so they cannot disagree again.
 */
export const MIN_NOTE_WIDTH = 2;

export function ticksToWidth(lenTicks: number, view: View): number {
  return Math.max(MIN_NOTE_WIDTH, (lenTicks / view.ppq) * view.zoom);
}

/** Semitones above C in each octave that are black keys. */
const BLACK_KEYS = new Set([1, 3, 6, 8, 10]);

/** Whether a pitch is a black key — what the gutter and the row banding draw. */
export function isBlackKey(pitch: number): boolean {
  return BLACK_KEYS.has(((pitch % 12) + 12) % 12);
}

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];

/**
 * A pitch's name, in scientific pitch notation.
 *
 * ⛔ MIDI 60 is C4 here, which is the convention Ableton, Logic and the MIDI
 * spec's own "middle C" use. FL Studio labels the same note C5. There is no
 * neutral choice, so this follows the majority and the one the rest of the
 * dataset's documentation already assumes — and it is written down here so the
 * next person does not "fix" it in one place and leave the gutter disagreeing
 * with the export.
 */
export function pitchName(pitch: number): string {
  const name = NOTE_NAMES[((pitch % 12) + 12) % 12];
  const octave = Math.floor(pitch / 12) - 1;
  return `${name}${octave}`;
}

/**
 * How near an edge a pointer must be to grab it rather than the note body.
 *
 * The roadmap asks for ≥ 6 px. It is also capped at a third of the note so a
 * short note does not become all edge and impossible to move — which is exactly
 * what happens to a 1/64 at low zoom if the zone is a flat 6 px.
 */
const EDGE_GRAB_PX = 6;

export function edgeAt(x: number, noteX: number, noteWidth: number): 'start' | 'end' | null {
  const zone = Math.min(EDGE_GRAB_PX, noteWidth / 3);
  if (x - noteX <= zone) return 'start';
  if (noteX + noteWidth - x <= zone) return 'end';
  return null;
}
