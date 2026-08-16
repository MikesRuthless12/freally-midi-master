import { create } from 'zustand';

import type { Complexity, Pattern, Scale, Song } from '../lib/ipc-types';

/**
 * The operation log undo and redo walk (FMM-U01).
 *
 * **Snapshots, not commands.** Every reversible action in this app is a write
 * to a handful of session fields, and the pattern those fields produce is
 * *derived* — `plugin/src/bridge.rs` regenerates it byte-for-byte from the same
 * seed. So an entry is the document state itself rather than a pair of do/undo
 * closures: there is nothing to invert, and a snapshot cannot drift from the
 * action that wrote it the way a hand-written inverse can.
 *
 * **Unlimited is affordable because of that.** A snapshot is five scalars, a
 * four-field pins object and one `Pattern` *reference* — the pattern is already
 * in memory and is shared, not copied, so a hundred undo steps across one
 * generation cost one pattern. Distinct patterns only accrue when the user
 * actually presses Generate, which is a deliberate act at human speed.
 *
 * ⛔ **This store knows nothing about the session store.** [`undo`] and [`redo`]
 * hand back a snapshot and the caller applies it. A history that reached into
 * `useSession` would be a cycle, and worse, two places that decide what a
 * session *is*.
 */

/** The session fields an undo step restores. Everything else is derived or transient. */
export type Snapshot = {
  selectedId: string | null;
  seed: string;
  /**
   * The record every part is written against (TASK-141).
   *
   * ⛔ **Restored with the clips, or undo puts them back under a plan they were
   * not written to.** Generate on Drake (record A), switch to Future and
   * generate (record B), then Ctrl+Z: the clips come back as Drake's, and
   * without this the record stays B — so the next Generate on Drake joins
   * Future's harmonic plan with nothing on screen saying so.
   */
  songSeed: string;
  /**
   * Whether `seed` is the producer's choice or the engine's echo.
   *
   * ⛔ In here because it is in `SAVED_FIELDS`, and this module's own notes on
   * `audioEnabled` and `mutedLanes` record what happens when a saved field
   * misses the snapshot: the change persists when made and is lost when undone.
   * Locking a seed and pressing Ctrl+Z has to hand it back.
   */
  seedPinned: boolean;
  bars: number;
  pins: {
    bpm: number | null;
    keyRoot: number | null;
    scale: Scale | null;
    swing: number | null;
    /** The clip's own meter, when the producer set one (TASK-041E). */
    timeSigNum: number | null;
    timeSigDen: number | null;
  };
  autoSync: boolean;
  /**
   * How busy a reading the producer asked for (TASK-125).
   *
   * ⛔ **In here for the same reason `seedPinned` is, and the compiler is what
   * caught it missing.** `SAVED_FIELDS_MATCH_SNAPSHOT` is a compile-time check
   * that the saved list and this type name the same fields — a field that saves
   * and does not undo is the exact drift it exists to prevent, and it reported
   * this one before any test ran. ⚠ The vitest case for it passed regardless,
   * because vitest strips types without checking them: a type error is not a
   * test failure, which is why `typecheck` is its own gate.
   */
  complexity: Complexity;
  /**
   * One clip per part (TASK-119).
   *
   * ⚠ **The shared-reference argument above still holds, and it is why this is a
   * map of references rather than a deep copy.** Regenerating one part rebuilds
   * the small outer object and keeps the other four `Pattern` references, so a
   * hundred undo steps across one generation still cost one pattern — not five.
   */
  patterns: Partial<Record<Pattern['part'], Pattern>>;
  /** Which parts are hand-edits — per-part since TASK-119's five slots. */
  editedParts: Pattern['part'][];
  /**
   * Whether the clip is an edit rather than the seed's own output (TASK-041).
   *
   * ⛔ In the snapshot because undoing back past the first edit has to make the
   * clip the seed's again. Left out, a project undone to its generated state
   * would still be saved as an edited one and reopen replaying a clip nobody
   * had edited — the small-file property lost with nothing to show for it.
   */
  edited: boolean;
  /** The pinned mood, or `null` for "Any" (TASK-040V). */
  mood: string | null;
  /** The genre to generate the artist in, or `null` for their own (TASK-158C). */
  base: string | null;
  /**
   * Whether the plugin sounds its own kit (FMM-S02).
   *
   * ⛔ In here because it is in `send()`: the undo stack and the saved session
   * carry the same fields, and this one shipped in the persist guard but not
   * the snapshot — so Ctrl+Z stepped everything else back and left the plugin
   * silent, with nothing saying why the undo was partial.
   */
  audioEnabled: boolean;
  /**
   * Lanes silenced in the preview (FMM-S02).
   *
   * ⛔ The same drift as `audioEnabled` above, one field over: `send()` has
   * carried this since the sampler landed, while the snapshot and the persist
   * guard did not — so a mute was saved when clicked and lost when undone.
   * **Every field `send()` carries belongs in all three places.** Replaced
   * wholesale on every change, so reference equality is enough to compare it.
   */
  mutedLanes: string[];
  /**
   * Lanes soloed in the preview (TASK-043).
   *
   * ⛔ Here for the reason the two fields above spell out, and it is the rule
   * rather than a case: **every field `send()` carries belongs in all three
   * places** — `SAVED_FIELDS`, this type and `snapshotOf`. Miss one and the
   * change is saved when made and lost when undone, which is worse than not
   * being undoable at all because the project and the screen then disagree.
   */
  soloedLanes: string[];
  /**
   * Lanes held across a reroll (TASK-044).
   *
   * Here for the rule the two fields above spell out: every field `send()`
   * carries belongs in `SAVED_FIELDS`, this type and `snapshotOf` alike.
   */
  lockedLanes: string[];
  /**
   * Generators switched off for playback (TASK-127).
   *
   * ⛔ Here for the same rule, and it arrived late: this lived in `ui.ts` and
   * was neither saved nor undoable, so an import that switched the bassline off
   * — because the file it came from has none — handed the switch back on when
   * the project was reopened, with `patterns.bass` still saved beside it. The
   * project then played a bass the imported record does not contain, which is
   * the failure the switch-off exists to prevent.
   */
  partsOff: Pattern['part'][];
  /**
   * The arrangement (TASK-063B).
   *
   * ⛔ **In the session's own snapshot rather than in a second stack, and that
   * is the reuse this module's header asks for.** A producer has one Ctrl+Z.
   * Two stacks would mean the shortcut had to decide which document it was
   * about — from the visible tab, which is the only clue available — and
   * undoing an arrangement edit after switching to the roll would then step the
   * *session* back instead. That is exactly what the Song tab used to do, and
   * why its undo was turned into a deliberate no-op rather than left wrong.
   *
   * Reference equality is what [`changed`] compares, and `clips.ts` returns the
   * *same* `Song` when an edit changes nothing — so a resize to the width it
   * already had records no step, without this module knowing what a resize is.
   */
  song: Song | null;
  /** Whether `song` is an arrangement rather than the seed's own output. */
  songEdited: boolean;
};

/**
 * Which field an entry changed, so the UI can name what it is about to undo
 * and so consecutive edits to one control collapse into one step.
 *
 * A stable identifier rather than a display string: the app ships 18 locale
 * catalogs, and a label baked in here would be a nineteenth place strings live.
 * The button maps this to a catalog key when it renders.
 */
export type Field = keyof Snapshot;

/**
 * Fields that never coalesce, however fast they arrive.
 *
 * ⛔ `mutedLanes` belongs here because a mute is a *discrete act*, not a value
 * being typed toward. Coalescing suits the seed box, where six keystrokes are
 * one intention; muting the kick and then the snare inside 600 ms is two, and
 * merging them made "kick muted, snare audible" unreachable by undo — one
 * Ctrl+Z un-muted both.
 */
const DISCRETE: readonly Field[] = [
  'patterns',
  'mutedLanes',
  'soloedLanes',
  'lockedLanes',
  'partsOff',
  'song',
];

/**
 * How long a run of edits to one field stays one undo step.
 *
 * Typing a six-digit seed is one action to the person typing it, and six undo
 * steps would be six presses of Ctrl+Z to get back where they started. The same
 * reasoning as the 300 ms save debounce in `session.ts`, with a longer window
 * because undo granularity is judged by intent rather than by write cost.
 */
const COALESCE_MS = 600;

/**
 * The most steps kept.
 *
 * ⛔ **Unlimited stopped being affordable when an entry started pinning a whole
 * arrangement.** The module header's argument still holds for everything else —
 * an entry is a handful of scalars and a *reference* to a shared pattern — but
 * a re-roll receives a freshly deserialized `Song` from the bridge with no
 * structural sharing at all, so every press of `R` retained a complete
 * independent copy of the record's note data for the life of the session,
 * inside somebody's DAW process. `song` is also in [`DISCRETE`], so those never
 * coalesce away.
 *
 * ⚠ **A thousand is a bound on the pathological case, not a limit on the
 * feature.** It is far past anything a producer reaches by hand in one sitting,
 * so the "unlimited undo" the README claims stays true of every session anybody
 * actually has — but it is finite, so a runaway cannot retain a DAW's memory
 * for the life of the process.
 */
const HISTORY_LIMIT = 1_000;

type Entry = {
  state: Snapshot;
  /** `null` when several fields moved at once — a preset load, or a generation. */
  field: Field | null;
  at: number;
};

type HistoryState = {
  past: Entry[];
  /** Where the document is now. `null` until [`arm`] establishes a baseline. */
  present: Entry | null;
  future: Entry[];

  /**
   * Start recording, with `state` as the point undo cannot go behind.
   *
   * ⛔ Called *after* the project restore, never before. A history armed at
   * construction would let the user undo the session the host just handed back
   * and land on an empty plugin, which reads as the project having failed to
   * load.
   */
  arm: (state: Snapshot) => void;

  /** Note that the document changed. A no-op write records nothing. */
  record: (state: Snapshot) => void;

  /**
   * Correct the entry that is already current, without pushing a new one.
   *
   * ⛔ **For a caller whose own `set` records a snapshot it is about to
   * invalidate.** `put()` applies a preset in one `set` — which fires the
   * recorder — and only afterwards clears the arrangement the preset does not
   * carry, so the entry it just filed named a record no longer on screen. One
   * Ctrl+Y then resurrected it alongside the preset's pins, a state nobody had
   * been in.
   *
   * ⚠ Not `arm`, which resets `past` and would make a preset load impossible to
   * undo at all, and not `record`, which would push a second entry and make it
   * two steps. Both are wrong in ways a producer would notice.
   */
  amend: (state: Snapshot) => void;

  /** The state to restore, or `null` when there is nothing to undo. */
  undo: () => Snapshot | null;
  redo: () => Snapshot | null;
};

/** Which single field differs, `null` for none, `'*'` for more than one. */
function changed(a: Snapshot, b: Snapshot): Field | null | '*' {
  let only: Field | null = null;

  for (const key of Object.keys(b) as Field[]) {
    // Reference equality throughout, including `pins`: every writer in
    // `session.ts` replaces that object rather than mutating it, which is the
    // same invariant the persistence subscriber already relies on.
    if (a[key] === b[key]) continue;
    if (only !== null) return '*';
    only = key;
  }

  return only;
}

export const useHistory = create<HistoryState>((set, get) => ({
  past: [],
  present: null,
  future: [],

  arm(state) {
    set({ past: [], present: { state, field: null, at: Date.now() }, future: [] });
  },

  record(state) {
    const { past, present } = get();
    // Unarmed: the restore is still running and nothing is undoable yet.
    if (present === null) return;

    const field = changed(present.state, state);
    if (field === null) return;

    const at = Date.now();
    const entry: Entry = { state, field: field === '*' ? null : field, at };

    // A fresh edit is a new branch — whatever was undone is no longer reachable.
    const coalesces =
      entry.field !== null &&
      entry.field === present.field &&
      !DISCRETE.includes(entry.field) &&
      at - present.at < COALESCE_MS;

    set({
      // ⛔ The oldest step is dropped rather than the newest refused: a stack
      // that stopped accepting entries would silently stop recording edits,
      // which is worse than being unable to walk back to last Tuesday.
      past: coalesces ? past : [...past, present].slice(-HISTORY_LIMIT),
      present: entry,
      future: [],
    });
  },

  amend(state) {
    const { present } = get();
    if (present === null) return;
    set({ present: { ...present, state } });
  },

  undo() {
    const { past, present, future } = get();
    if (present === null || past.length === 0) return null;

    const previous = past[past.length - 1];
    // ⛔ `field: null` on the restored entry, deliberately. A restored entry
    // carries its *original* timestamp, so an edit to the same control shortly
    // after an undo would satisfy the coalescing window against a moment that
    // may be minutes old — replacing the entry instead of pushing, and making
    // the state just undone to unreachable. Landing on a step is a boundary.
    set({
      past: past.slice(0, -1),
      present: { ...previous, field: null },
      future: [present, ...future],
    });
    return previous.state;
  },

  redo() {
    const { past, present, future } = get();
    if (present === null || future.length === 0) return null;

    const [next, ...rest] = future;
    set({ past: [...past, present], present: { ...next, field: null }, future: rest });
    return next.state;
  },
}));

/** What the buttons and the shortcuts ask before they offer themselves. */
export function canUndo(state: HistoryState): boolean {
  return state.present !== null && state.past.length > 0;
}

export function canRedo(state: HistoryState): boolean {
  return state.present !== null && state.future.length > 0;
}
