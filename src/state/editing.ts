import { create } from 'zustand';

import type { NoteId, Snap } from '../components/PianoRoll/notes';

/**
 * What the piano roll is looking at and what is selected (TASK-041).
 *
 * ⛔ **Deliberately not part of the undo snapshot, and not saved with the
 * project.** Zoom, scroll and the selection are where the user is *standing*,
 * not what they made. `state/history.ts` records a snapshot on every store
 * write, so putting the selection in there would make a rubber-band drag across
 * forty notes forty undo steps deep — and Ctrl+Z would then spend those forty
 * presses restoring selections before it reached the edit the user wanted back.
 * Ableton and FL both treat selection the same way.
 *
 * ⛔ **The live drag also lives here rather than in the pattern.** A pointermove
 * fires at screen rate; committing each one to `useSession.pattern` would push
 * an undo entry per frame, and `history.ts` lists `pattern` as *discrete* so
 * they would not even coalesce. So a drag is held as a delta and applied once on
 * pointerup — which is also what makes "drag it back where it started" cost
 * nothing.
 */

/** The default row height the roadmap specifies for the roll. */
export const DEFAULT_ROW_HEIGHT = 20;

/** How far the roll may be zoomed, as pixels per quarter note. */
export const MIN_ZOOM = 8;
export const MAX_ZOOM = 512;
const DEFAULT_ZOOM = 64;

/** Row heights the zoom walks between, so a row is never a fractional pixel. */
export const MIN_ROW_HEIGHT = 8;
export const MAX_ROW_HEIGHT = 48;

/**
 * A drag in progress.
 *
 * `kind` decides what the delta means, which is why one shape covers all of
 * them: the renderer draws a preview from the same fields whichever gesture is
 * running, so there is one place a preview can disagree with what commits.
 */
export type Drag =
  | { kind: 'move'; deltaTicks: number; deltaPitch: number; copy: boolean }
  | { kind: 'resize'; edge: 'start' | 'end'; deltaTicks: number }
  | { kind: 'marquee'; fromTick: number; toTick: number; fromPitch: number; toPitch: number }
  /**
   * A velocity gesture in the lane (TASK-041V), as a value per *stem* id.
   *
   * ⛔ A map rather than "these ids, this value", because a paint across the
   * lane writes a different number to every slider it passes — that is the
   * whole gesture. One shape covers the flat drag, the `Shift` ramp and the
   * relative move of a selection, so the roll's live re-shading reads exactly
   * what the commit will write.
   */
  | { kind: 'velocity'; values: Readonly<Record<string, number>> };

type EditingState = {
  /** The grid every gesture rounds to. */
  snap: Snap;
  /** Pixels per quarter note. The horizontal zoom. */
  zoom: number;
  /** Pixels per semitone row. The vertical zoom. */
  rowHeight: number;
  /** Leftmost visible tick. */
  scrollTick: number;
  /** Topmost visible pitch — rows count *down* from here. */
  topPitch: number;
  /**
   * The notes under edit, by [`NoteId`].
   *
   * Replaced wholesale rather than mutated, so a component can subscribe by
   * reference and re-render only when the selection actually changes.
   */
  selection: ReadonlySet<NoteId>;
  /** The gesture in flight, or `null` between gestures. */
  drag: Drag | null;
  /**
   * Hide every row that is not in the key (TASK-041B).
   *
   * ⛔ **A view transform, and nothing else.** The notes do not move, a hidden
   * row still plays, and it still exports — folding is about what is on screen.
   * An out-of-scale note in a folded roll is therefore *audible and invisible*,
   * which is why the roll draws a marker for it rather than letting it vanish.
   */
  foldToScale: boolean;
  /**
   * Ableton's `Fold`: show only rows that currently have notes.
   *
   * Independent of `foldToScale` — one asks "is this row in the key", the other
   * "is anything written on it" — and both may be on, which is the useful case
   * for a dense clip in an unfamiliar scale.
   */
  foldToNotes: boolean;
  /**
   * How hard `Quantize` pulls towards the grid (TASK-041D).
   *
   * A blend rather than a switch, and it lives here so it survives closing the
   * menu — a producer who set 60% once meant it for the session, not for one
   * press. The same blend `engine::humanize`'s `quantize_strength` applies.
   */
  quantizeStrength: number;

  setSnap: (snap: Snap) => void;
  setZoom: (zoom: number) => void;
  /**
   * Show a whole clip at once, however long it is.
   *
   * ⛔⛔ **Mike, 2026-08-09**: *"the entire midi piano roll pattern needs to be
   * shown, not just most of it, whether it's 4 or 8 bars shouldn't matter."*
   * The roll opened at a fixed 64px per quarter, so a four-bar clip needed
   * 1024px and an eight-bar one needed 2048 — anything past the stage width was
   * simply off-screen, and a producer had to zoom out by hand every time to see
   * what had been generated. Length is a property of the clip; the zoom should
   * follow it rather than the other way round.
   *
   * ⚠ Clamped to `MIN_ZOOM`, so an absurdly long clip stops shrinking rather
   * than becoming an unreadable smear — past that it scrolls, which is the
   * honest answer.
   */
  fitTo: (totalTicks: number, ppq: number, viewportPx: number) => void;
  setRowHeight: (height: number) => void;
  scrollTo: (tick: number, pitch: number) => void;

  select: (ids: Iterable<NoteId>) => void;
  addToSelection: (ids: Iterable<NoteId>) => void;
  toggleSelected: (id: NoteId) => void;
  clearSelection: () => void;

  setFoldToScale: (on: boolean) => void;
  setFoldToNotes: (on: boolean) => void;
  setQuantizeStrength: (strength: number) => void;

  /**
   * Start, update or clear the gesture in flight.
   *
   * One setter rather than the three this had. `beginDrag` and `updateDrag`
   * were byte-identical, which is two names for one operation and an invitation
   * to believe they differ.
   */
  setDrag: (drag: Drag | null) => void;
};

export const useEditing = create<EditingState>((set, get) => ({
  snap: '1/16',
  zoom: DEFAULT_ZOOM,
  rowHeight: DEFAULT_ROW_HEIGHT,
  scrollTick: 0,
  // C3 at the top of a 20px-row viewport puts a typical melody register in the
  // middle of the screen without anyone reaching for the scrollbar first.
  topPitch: 84,
  selection: new Set<NoteId>(),
  drag: null,
  foldToScale: false,
  foldToNotes: false,
  // Hard by default, which is what "quantize" means to anyone who has not
  // opened the slider yet.
  quantizeStrength: 1,

  setSnap: (snap) => set({ snap }),
  setZoom: (zoom) => set({ zoom: Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom)) }),

  fitTo: (totalTicks, ppq, viewportPx) => {
    // Nothing to fit to, and dividing by either would give Infinity.
    if (totalTicks <= 0 || ppq <= 0 || viewportPx <= 0) return;
    const quarters = totalTicks / ppq;
    // ⚠ A hair under the full width, so the clip's last bar line is inside the
    // viewport rather than exactly on its edge where it reads as cut off.
    const wanted = (viewportPx * 0.985) / quarters;
    set({
      zoom: Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, wanted)),
      // ⛔ Back to the start with it. Fitting a clip while scrolled into the
      // middle of the previous one shows the right *amount* of a clip and the
      // wrong *part* of it.
      scrollTick: 0,
    });
  },
  setRowHeight: (rowHeight) =>
    set({
      rowHeight: Math.min(MAX_ROW_HEIGHT, Math.max(MIN_ROW_HEIGHT, Math.round(rowHeight))),
    }),

  scrollTo: (tick, pitch) =>
    set({
      scrollTick: Math.max(0, tick),
      // Clamped to MIDI's range rather than to the notes present: scrolling to
      // an empty register is how a note gets drawn there in the first place.
      topPitch: Math.min(127, Math.max(0, Math.round(pitch))),
    }),

  select: (ids) => set({ selection: new Set(ids) }),

  addToSelection: (ids) => {
    const next = new Set(get().selection);
    for (const id of ids) next.add(id);
    set({ selection: next });
  },

  toggleSelected: (id) => {
    const next = new Set(get().selection);
    if (!next.delete(id)) next.add(id);
    set({ selection: next });
  },

  clearSelection: () => {
    // Guarded so `Esc` on an empty selection does not hand every subscriber a
    // fresh empty Set and re-render the roll for nothing.
    if (get().selection.size === 0) return;
    set({ selection: new Set<NoteId>() });
  },

  setFoldToScale: (foldToScale) => set({ foldToScale }),
  setFoldToNotes: (foldToNotes) => set({ foldToNotes }),
  setQuantizeStrength: (strength) =>
    set({ quantizeStrength: Math.min(1, Math.max(0, strength)) }),

  setDrag: (drag) => set({ drag }),
}));
