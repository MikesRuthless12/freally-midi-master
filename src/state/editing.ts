import { create } from 'zustand';

import type { NoteId, Snap } from '../components/PianoRoll/notes';
import type { Lane } from '../lib/ipc-types';

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
 * How far a lane's window may be panned from its root, in semitones.
 *
 * ⚠ **A backstop on an unbounded integer, and nothing more.** The real limit is
 * MIDI's own 0–127, applied where the drag happens — this only has to be wide
 * enough that it can never bite first. ⛔ It was 60, and 60 was too small: a
 * `sub` lane rooted at 36 needs a pan of +91 to reach pitch 127, so the clamp
 * would have stopped the window following the note before MIDI did, which is
 * the one thing panning exists to do.
 */
const MAX_LANE_WINDOW_PAN = 127;

/** Where one open lane's seven-row window is looking. */
export type LaneWindow = {
  /**
   * The pitch the rows are measured from, frozen when the lane opened.
   *
   * ⛔⛔ **Frozen, and that is a correctness fix rather than an optimisation.**
   * The root is the lane's most common pitch, so in a lane holding one hit it is
   * *that hit* — and recomputing it per render meant dragging the only note up
   * two semitones moved the root up two semitones with it, re-centred the
   * window, and drew the note back in the middle. The note moved and the screen
   * said it had not. Freezing at open time makes the rows a fixed ruler that
   * edits move against, which is what a producer is looking at them for.
   */
  root: number;
  /**
   * How far the window has been slid from that root, in semitones.
   *
   * ⛔ **Per lane, deliberately.** The kick can sit at its root while the 808 is
   * panned five semitones up; one shared offset would move every open lane at
   * once, which is not what "each row has its own pitch" means.
   */
  pan: number;
};

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
  /**
   * The drum lanes expanded into a pitch lane, and where each one is looking
   * (TASK-161). A lane is open exactly when it has an entry here.
   *
   * ⛔ **Every lane may expand, not a chosen few.** Mike, 2026-08-12: *"i want
   * all the lanes to have the ability to expand into a pitch lane."* The
   * mechanism is the same whatever the lane — a hit's pitch against the lane's
   * root — so an allowlist would be more code than uniformity and would raise
   * the question of why the cowbell is exempt. The 808 is merely the loudest
   * case: `sub` and `subLow` carry musical pitch, and every other lane reads its
   * offset as the sample transposition `pitch - gm_drum_note(lane)`, which is
   * already how `snareRoll.pitchWalk` climbs a roll.
   *
   * ⚠ **Empty by default**, so a producer who never opens one sees the grid they
   * had before this existed.
   *
   * ⛔ **One map rather than a Set and two records.** The three were only ever
   * written together and only ever read together, and splitting them made
   * "expanded" and "has a root" two facts that could disagree — plus two `??`
   * fallbacks for a state that cannot occur.
   */
  openLanes: Readonly<Partial<Record<Lane, LaneWindow>>>;

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
  /**
   * Show a whole clip's *register* at once — the vertical half of `fitTo`.
   *
   * ⛔⛔ **Mike, 2026-08-09**: *"ensure that all notes that have been generated
   * are visible, not just most of them, i don't care if you need to zoom out a
   * little."* `fitTo` fixed the horizontal half only: it sets `zoom` and
   * `scrollTick` and never touches `rowHeight`, so a melody spanning two octaves
   * needed 24 rows and a 400 px stage at the default 20 px row shows 20. The
   * notes off the top and bottom were as invisible as the off-screen bars were,
   * and for the same reason.
   *
   * ⚠ **Shrinks the row to fit, and grows it back no further than the default.**
   * Clamping the upper end to the current height would leave a roll that had
   * once framed a wide clip stuck at 8 px rows forever; clamping it to
   * `MAX_ROW_HEIGHT` would blow a three-note bass line up to 48 px rows. The
   * default is the height a producer expects to open on.
   *
   * ⚠ **Measured in semitones, not in visible rows.** A fold removes rows from
   * the middle, so the folded row count is never *more* than the semitone span —
   * fitting the span therefore fits the fold too, a little smaller than it had
   * to be. The reverse assumption would crop.
   */
  frameTo: (highest: number, lowest: number, viewportPx: number) => void;
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
   * Open or close a lane's pitch rows (TASK-161).
   *
   * `root` is the pitch the window freezes around, and the caller supplies it
   * because it is read off the pattern — this store does not see patterns, and
   * `engine::midi::gm_drum_note` is deliberately not mirrored on the page.
   */
  toggleLaneExpanded: (lane: Lane, root: number) => void;
  /**
   * Slide a lane's window up or down by `semitones`.
   *
   * ⛔ **This is what makes travel unbounded while the row stays seven tall.**
   * Dragging a hit past the top or bottom row pans rather than stopping, so the
   * reachable range is all of MIDI — the window is a viewport onto the lane's
   * pitch, not a limit on it.
   */
  panLaneWindow: (lane: Lane, semitones: number) => void;
  /**
   * Re-measure every open lane against a new pattern (TASK-161).
   *
   * ⛔⛔ **A frozen root belongs to the pattern it was frozen against.** Nothing
   * closed these on Generate, so a lane left open across one kept yesterday's
   * root: open `sub` on a track rooted at 36, generate one rooted at 45, and all
   * seven rows drew empty with an edge marker on every column — a lane full of
   * hits presented as a lane with none.
   *
   * `roots` carries the new root per lane, read off the new pattern by the
   * caller; a lane whose entry is missing or `null` has no hits to measure and
   * closes rather than showing a window around a pitch that is not there.
   */
  refreezeOpenLanes: (roots: Readonly<Partial<Record<Lane, number | null>>>) => void;

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
  openLanes: {},

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
  frameTo: (highest, lowest, viewportPx) => {
    if (viewportPx <= 0 || highest < lowest) return;
    // A row of air above the top note and below the bottom one, so neither sits
    // flush against the ruler or the bottom edge where it reads as cut off —
    // the vertical twin of `fitTo`'s 0.985.
    const wanted = highest - lowest + 3;
    const rowHeight = Math.min(
      DEFAULT_ROW_HEIGHT,
      Math.max(MIN_ROW_HEIGHT, Math.floor(viewportPx / wanted)),
    );
    const rows = Math.max(1, Math.floor(viewportPx / rowHeight));
    // ⛔⛔ **Centred, always — the leftover space is SPLIT rather than dumped
    // below the notes.** Anchoring the top at `highest + 1` is only the right
    // answer when the range exactly fills the stage, and it almost never does:
    // the row height is clamped up to `DEFAULT_ROW_HEIGHT`, so a five-note 808
    // line in a 500 px roll got 25 rows for a range needing 8 — five rows jammed
    // against the ruler with four hundred pixels of empty keyboard underneath.
    // The centring this replaced (`middle + rows / 2`) did not have that
    // failure, and losing it was a regression dressed as a simplification.
    //
    // ⚠ It also covers the case the old comment was about: when the range is
    // taller than the stage even at `MIN_ROW_HEIGHT`, centring shows the middle
    // of it, which is the honest answer rather than a top edge with the rest
    // below the fold. One expression, both cases.
    const middle = (highest + lowest) / 2;
    set({
      rowHeight,
      // `(rows - 1) / 2` rather than `rows / 2`: the visible band runs from
      // `topPitch` *down to* `topPitch - rows + 1`, so its centre is half a row
      // below the top edge, not a whole one.
      topPitch: Math.min(127, Math.max(0, Math.round(middle + (rows - 1) / 2))),
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

  toggleLaneExpanded: (lane, root) =>
    set((state) => {
      // Replaced wholesale rather than mutated, for the reason `selection`
      // gives above: a component that subscribes by reference has to see it.
      const openLanes = { ...state.openLanes };
      if (openLanes[lane]) {
        delete openLanes[lane];
        return { openLanes };
      }
      // ⚠ Opening re-freezes the root and re-centres the pan, so a lane always
      // opens showing where its hits actually are. Carrying yesterday's pan into
      // a freshly generated pattern would open on seven empty rows.
      openLanes[lane] = { root, pan: 0 };
      return { openLanes };
    }),

  panLaneWindow: (lane, semitones) =>
    set((state) => {
      const open = state.openLanes[lane];
      // Panning a closed lane is not a thing the grid can ask for, and writing
      // an entry here would silently open one.
      if (!open) return {};

      const pan = Math.min(
        MAX_LANE_WINDOW_PAN,
        Math.max(-MAX_LANE_WINDOW_PAN, open.pan + semitones),
      );
      // ⚠ A pan that changes nothing must not write, or holding a drag against
      // the clamp would publish a new `openLanes` object every pointermove and
      // re-render every open lane at screen rate.
      if (pan === open.pan) return {};
      return { openLanes: { ...state.openLanes, [lane]: { ...open, pan } } };
    }),

  refreezeOpenLanes: (roots) =>
    set((state) => {
      const open = Object.keys(state.openLanes) as Lane[];
      if (open.length === 0) return {};

      const openLanes: Partial<Record<Lane, LaneWindow>> = {};
      for (const lane of open) {
        const root = roots[lane];
        if (root === null || root === undefined) continue;
        // ⚠ The pan resets with the root. Carrying an offset measured against a
        // different pattern's root is the same bug one level down.
        openLanes[lane] = { root, pan: 0 };
      }
      return { openLanes };
    }),

  setDrag: (drag) => set({ drag }),
}));
