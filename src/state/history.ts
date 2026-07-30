import { create } from 'zustand';

import type { Pattern, Scale } from '../lib/ipc-types';

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
  bars: number;
  pins: {
    bpm: number | null;
    keyRoot: number | null;
    scale: Scale | null;
    swing: number | null;
  };
  autoSync: boolean;
  pattern: Pattern | null;
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

/** Fields that never coalesce, however fast they arrive. */
const DISCRETE: readonly Field[] = ['pattern'];

/**
 * How long a run of edits to one field stays one undo step.
 *
 * Typing a six-digit seed is one action to the person typing it, and six undo
 * steps would be six presses of Ctrl+Z to get back where they started. The same
 * reasoning as the 300 ms save debounce in `session.ts`, with a longer window
 * because undo granularity is judged by intent rather than by write cost.
 */
const COALESCE_MS = 600;

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
      past: coalesces ? past : [...past, present],
      present: entry,
      future: [],
    });
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
