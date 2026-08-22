/**
 * The preview kit, and the producer's own one-shots over it (TASK-131B).
 *
 * ⛔ **This store exists because the KIT panel was connected to nothing.**
 * `RightRail` rendered eight hardcoded disabled buttons and a static "No kit
 * yet" while a twelve-pad kit was loaded and audibly playing — TASK-136, found
 * by Mike in Ableton within a minute of opening it. A panel that lies is worse
 * than a panel that is missing, because it answers the question wrongly instead
 * of leaving it open. Everything drawn there now comes from `kit_state`.
 */

import { create } from 'zustand';

import { invoke } from '../lib/ipc';
import { noteDocumentChange, reason, registerKitDocument, useSession } from './session';
// ⚠ A store-to-store read, not a subscription: the re-roll asks where the
// browser is standing at the moment it fires, and nothing here re-renders on it.
import { standingIn, useExplorer } from './explorer';
// ⚠ Another store-to-store read, for the same reason: opening the editor has to
// bring the panel that draws it on screen, and nothing here re-renders on the
// rail's layout.
import { useUi } from './ui';
import type { Lane } from '../lib/ipc-types';

// ⚠ Re-exported so every existing importer keeps working; the list itself
// lives in a leaf module that a Playwright spec can also import. See
// `state/lanes.ts` for why that mattered.
export { ALL_LANES } from './lanes';

/**
 * Can this lane actually make a sound?
 *
 * ⛔ **One predicate, because the same question was being asked two ways.**
 * `DragRows` asked `shipped || path !== null` to decide whether to offer an
 * Audio handle; `KitPanel` asked `!shipped && name === null` to dim a row — the
 * same concept keyed on *different fields*. Nothing made them move together, so
 * a lane could be drawn as playable in one panel and hidden in the other, with
 * no test able to catch the disagreement because neither file knew about the
 * other.
 *
 * ⚠ A one-shot counts. The producer's own sample is a sample, and
 * `audio/render.rs` renders a file from either.
 */
export function canSound(lane: KitLane): boolean {
  return lane.shipped || lane.path !== null;
}

/**
 * Every lane the loaded kit can actually make a sound with.
 *
 * ⚠ **Here rather than memoised twice**, for the same reason [`canSound`] is one
 * predicate: `DragRows` builds this to decide whether a part offers an Audio
 * handle and `SongTimeline` builds it to decide whether a clip does, and two
 * copies of "which lanes are audible" is how the Stems panel and the arrangement
 * come to disagree about one kit. Both memoise the call on `lanes`.
 */
export function soundableLanes(lanes: KitLane[]): Set<Lane> {
  return new Set(lanes.filter(canSound).map((lane) => lane.lane));
}

/**
 * The pad's own amplitude envelope (TASK-164).
 *
 * ⚠ **Sustain is in dB, not a 0–1 ratio** — read off the reference Mike
 * supplied: *"A 0.00 ms · D 195 ms · S −36.00 dB · R 5.00 s"*.
 */
export type Adsr = {
  attackMs: number;
  decayMs: number;
  sustainDb: number;
  releaseMs: number;
};

/**
 * What the producer did to a pad, as distinct from what the kit shipped
 * (TASK-055A, TASK-164).
 *
 * ⛔⛔ **Hand-written, and nothing here ever constructs one from scratch.**
 * `ts-rs` exports the engine's IPC types and deliberately skips this one — two
 * test binaries writing a single `ipc-types.ts` under a parallel
 * `cargo test --workspace` is a race, and that file is already gated by a
 * `git diff --exit-code` in `ci:local`.
 *
 * ▶ **So the plugin owns the defaults outright and this file never guesses
 * them.** `kit_state` sends a full `tweaks` block for *every* lane, including
 * the ones nobody has touched, and every edit below is a copy of what arrived
 * with one field changed. That is what stops this becoming the
 * `state/lanes.ts` failure — a comment claiming to mirror something, drifted to
 * 21 of 37, with two green tests watching. The names are pinned on the Rust
 * side by `the_wire_names_the_page_binds_to_do_not_move_silently`.
 */
export type PadTweaks = {
  gainDb: number;
  /** -1 hard left, 0 centre, +1 hard right. */
  pan: number;
  semis: number;
  cents: number;
  normalize: boolean;
  /** Where the pad starts playing, as a fraction of the sample. */
  trimStart: number;
  /** Where it stops. `1` is the whole sample. */
  trimEnd: number;
  fadeInS: number;
  fadeOutS: number;
  adsr: Adsr;
};

/** One row of the KIT panel. Mirrors what `kit_state` answers with. */
export type KitLane = {
  lane: Lane;
  /**
   * Whether the *shipped* kit has a voice for this lane.
   *
   * ⚠ `false` means the lane is silent unless a one-shot is assigned. `snap` is
   * that lane today: the drum generator can write it and no shipped pad has
   * ever played it.
   */
  shipped: boolean;
  /** The file name of the producer's own sample, or `null` for the shipped one. */
  name: string | null;
  path: string | null;
  /**
   * This pad's edits, always present.
   *
   * ⚠ **Never `null`, even for a lane nobody has opened** — the plugin sends
   * its own defaults. A nullable field here would make every untouched pad a
   * special case in the one place a control has to have a position.
   */
  tweaks: PadTweaks;
  /**
   * Whether this pad's buffer was flipped at decode (`Ctrl`+←).
   *
   * ⛔ **The editor needs it to draw the trim handles over the right end.**
   * `oneshot::load` bakes the reversal into the samples, so the trim window is
   * measured against the REVERSED audio — while `explorer_waveform` reads the
   * file off disk forwards. Drawing one and cutting the other is a picture that
   * lies about what the handles do.
   */
  reversed: boolean;
  /**
   * What note this pad's sample was measured to be in (TASK-052).
   *
   * ⚠ **`null` is three different real answers**, and the panel says so rather
   * than printing a note nobody measured: a drum lane (no root applies), a
   * shipped pad (nothing of the producer's on it), and a sample with no clear
   * pitch — a vocal chop or a noisy pad, which `detect_root` refuses rather
   * than guessing at.
   */
  root: DetectedRoot | null;
  /**
   * Whether a held note on this pad actually holds (TASK-053A).
   *
   * ⚠ **Three states, and the middle one is the point.** `null` is a lane
   * where holding means nothing — a drum — or a lane with nothing of the
   * producer's on it. `true` is a sample with a steady state to loop.
   * `false` is the one that earns a sentence on screen: the note will end
   * when the file does, and saying so beats shortening it silently.
   */
  holds: boolean | null;
};

/**
 * A measured root, with how sure the measurement is.
 *
 * ⛔ **The clarity travels with the note because TASK-052 asks that the
 * confidence be *surfaced*.** A root found at 0.61 on a noisy chop is a number
 * a producer should be allowed to distrust — and the transpose dial beside it
 * is what they fix it with.
 */
export type DetectedRoot = {
  /** MIDI note number. */
  note: number;
  /** How far off that note it actually is, in cents. `-50..=50`. */
  cents: number;
  /** NSDF peak height, `0..=1`. Higher is more clearly one pitch. */
  clarity: number;
};

type KitStateReply = {
  id: string | null;
  lanes: KitLane[];
};

/**
 * The producer's own samples: which file is on which lane, and which way round
 * (TASK-050A).
 *
 * ⛔ **The direction is carried, not derived.** `oneshot::load` bakes a
 * reversal into the buffer, so a path alone does not describe the sound — an
 * undo that restored the path and dropped the flag would silently un-reverse a
 * pad the producer had reversed on purpose, and `apply` would then persist that
 * loss. Lanes with nothing of theirs on them are absent, which is what the
 * plugin reads as *"put the built-in sound back"*.
 */
export type AssignedKit = Record<string, { path: string; reversed: boolean }>;

/**
 * One file a batch import did not place, and why. Mirrors `oneshot::Refused`.
 *
 * ⚠ `reason` is the plugin's own English, like every other message that reaches
 * `error` — it names a decode failure, not a UI state, and there is nothing on
 * this side that could translate it without inventing a code for each one.
 */
export type Refused = {
  name: string;
  reason: string;
};

/** What the last batch import did (TASK-049). */
export type ImportReport = {
  loaded: number;
  refused: Refused[];
};

/** What `one_shot_status` answers with. Mirrors `oneshot::Status`. */
type OneShotStatus =
  | { state: 'idle' }
  | { state: 'running' }
  | { state: 'done'; lane: Lane; name: string }
  | { state: 'cancelled' }
  | { state: 'failed'; reason: string }
  | ({ state: 'imported' } & ImportReport)
  | { state: 'restored'; refused: Refused[] };

/**
 * How often the assignment poll asks.
 *
 * Slow on purpose: a human is browsing their sample folder, and the same
 * reasoning as `EXPORT_POLL_MS` applies — there is no event to wait on, because
 * the dialog is modal on a thread of its own.
 */
export const ONE_SHOT_POLL_MS = 400;

/**
 * The one poll of `one_shot_status` currently in flight, or `null`.
 *
 * ⛔ **Module state rather than a store field, and the distinction is real.**
 * Nothing renders from it — it exists so a second caller of [`awaitLoader`] does
 * not open a second reader of a mailbox that clears on read. A store field would
 * put a `Promise` in the undo snapshot's neighbourhood and re-render every
 * subscriber twice per assignment for something no component draws.
 *
 * ⚠ Reset by the promise's own `finally`, so a thrown poll cannot wedge every
 * later caller onto a promise that has already settled.
 */
let loading: Promise<void> | null = null;

/**
 * The producer's own samples, lane to path — what an undo step carries.
 *
 * ⚠ Only the lanes holding a file of theirs. A shipped pad has no path and is
 * absent, which is exactly what `one_shot_set_all` reads as *"put the built-in
 * sound back on this one"*.
 */
function oneShotsOf(lanes: KitLane[]): AssignedKit {
  const out: AssignedKit = {};
  for (const row of lanes) {
    if (row.path !== null) out[row.lane] = { path: row.path, reversed: row.reversed };
  }
  return out;
}

/**
 * Whether two kits name the same file, the same way round, on the same lanes.
 *
 * ⛔ **The direction is part of the comparison.** `oneshot::load` bakes a
 * reversal into the buffer, so the same path played backwards is a different
 * sound — treating the two as equal would let an undo across a `Ctrl`+← skip
 * the restore entirely and leave the pad playing forwards.
 */
function sameKit(a: AssignedKit, b: AssignedKit): boolean {
  const keys = Object.keys(a);
  return (
    keys.length === Object.keys(b).length &&
    keys.every(
      (lane) => a[lane].path === b[lane]?.path && a[lane].reversed === b[lane]?.reversed,
    )
  );
}

type KitState = {
  /** The loaded kit's id, or `null` before the first read / if none loaded. */
  id: string | null;
  lanes: KitLane[];
  /** Whether `kit_state` has ever answered. Distinguishes "empty" from "not asked". */
  loaded: boolean;
  /** The lane whose dialog is open, or `null`. */
  assigning: Lane | null;
  /**
   * Whether a batch import has the dialog slot (TASK-049).
   *
   * ⛔ **Not a lane parked in `assigning`.** A batch belongs to no lane, and
   * `KitPanel` draws *"choosing…"* over whichever row `assigning` names — so a
   * stand-in lane made that row stop showing its own sample for as long as the
   * dialog was open. Ask [`busy`] when the question is *"is a dialog open"*.
   */
  importing: boolean;
  /** Whether anything holds the plugin's single dialog slot. */
  busy: () => boolean;
  /** Put a saved kit on, and record it as one undo step (TASK-051/050A). */
  loadSaved: (id: string) => Promise<void>;
  /**
   * The lane whose sound editor is open, or `null` (TASK-059, TASK-164).
   *
   * ⛔ **In the store rather than inside `KitPanel`, because three gestures
   * open it and only one of them is in that file.** A sample lands on a lane
   * from the KIT row, from a pad in the grid, and from the grid's *"use
   * selected"* button — TASK-059 asks that each of them *"assigns, and opens
   * the per-one-shot editor"*, and a `useState` in the panel could only ever
   * serve the first.
   *
   * ⚠ One at a time. Two open editors would be two sets of controls over one
   * audio thread.
   */
  editingPad: Lane | null;
  /** The last thing that went wrong, for the panel to show. */
  error: string | null;
  /**
   * What the last batch import did, or `null` (TASK-049).
   *
   * ⛔ **Its own slot rather than `error`, because a batch that mostly worked
   * is not an error.** Eighteen of twenty landing is a *success* with two
   * things to tell the producer, and folding it into the one-line `error`
   * would either shout at them about a working import or throw away the names
   * of the two files that did not — which is the whole of what TASK-049 asks
   * for: *"per-file error toasts that never abort the batch"*.
   */
  imported: ImportReport | null;
  /**
   * The producer's own samples, lane to path — the undo snapshot's copy of the
   * kit (TASK-050A).
   *
   * ⛔ **Derived from `lanes` rather than a second source of truth.** The
   * plugin owns the kit; this is the shape `history` can compare and
   * `one_shot_set_all` can be handed back. `refresh` keeps the *same object*
   * whenever the assignments have not moved — see the note there for why that
   * is load-bearing.
   */
  oneShots: AssignedKit;
  refresh: () => Promise<void>;
  assign: (lane: Lane) => Promise<void>;
  /**
   * Pick a whole selection and let each file's name choose its pad (TASK-049).
   *
   * The plugin answers as soon as its loader thread is running, so this waits
   * on [`awaitLoader`] like every other gesture that opens a dialog.
   */
  addMany: () => Promise<void>;
  /**
   * Put a browsed sample on a lane, refresh, and record one undo step.
   *
   * ⛔ **The one door for the five gestures that do this** — see the action for
   * why it is not left to each of them. Answers whether the drop landed, so a
   * caller that opens the pad editor does not open it over a refusal.
   */
  drop: (lane: Lane, path: string, reversed?: boolean) => Promise<boolean>;
  /** Put the import report away once it has been read. */
  dismissImport: () => void;
  clear: (lane: Lane) => Promise<void>;
  /**
   * Re-roll pads from the folder the browser is showing (TASK-050A).
   *
   * ⛔ **The locked lanes are filtered out *here*, not in the plugin.** A locked
   * pad is exempt — TASK-044's rule applied to pads — and `lockedLanes` is the
   * page's state; sending the plugin a second copy of it is how the two would
   * start disagreeing about what is locked.
   *
   * `null` re-rolls every unlocked lane; a lane re-rolls just that one, and is
   * refused if that lane is locked rather than silently doing nothing.
   */
  randomize: (lane: Lane | null) => Promise<void>;
  /**
   * Change one pad's edits, and hear it (TASK-055A, TASK-164).
   *
   * ⛔ **The row is updated here BEFORE the plugin answers, and that is not
   * optimism for its own sake.** These are dragged controls: a knob whose
   * position only moves once a round trip has completed does not follow the
   * pointer, and the producer is dragging to *hear* the change. The plugin
   * rebuilds the kit and republishes it on the same call, so the sound and the
   * knob move together; a failure puts the row back and says why.
   *
   * ⚠ `patch` is a partial, and it is applied over **what the plugin last
   * sent** rather than over a default — see [`PadTweaks`] for why this file
   * never constructs one.
   */
  setTweaks: (lane: Lane, patch: Partial<PadTweaks>) => Promise<void>;
  /**
   * Open a lane's sound editor, or close whatever is open.
   *
   * ⚠ **Brings the KIT panel on screen too.** Two of the three gestures that
   * call this are in the *pad grid*, which is on the stage — setting state a
   * rail panel draws and leaving that panel closed would be an editor that
   * opened where nobody could see it.
   */
  editPad: (lane: Lane | null) => void;
  /**
   * Wait for the loader thread to finish, then refresh and surface any failure.
   *
   * ⛔ Shared by the re-roll and by loading a saved kit, because both hand the
   * work to that thread and answer immediately — a resolved promise from either
   * means "started", not "done".
   */
  awaitLoader: () => Promise<void>;
};

export const useKit = create<KitState>((set, get) => ({
  id: null,
  lanes: [],
  loaded: false,
  assigning: null,
  importing: false,
  editingPad: null,
  error: null,
  imported: null,
  oneShots: {},

  /**
   * Re-read what plays each lane.
   *
   * ⛔ **Does not clear `error` on success, and that is the fix rather than an
   * omission.** Every assignment ends by refreshing, so clearing here wiped the
   * one message that explains what went wrong: a producer whose sample was
   * refused as silent saw the refusal replaced by nothing, a fraction of a
   * second later, and was left with a lane that had simply not changed. Errors
   * are cleared by the action that starts — `assign` and `clear` — because that
   * is the point at which the old one stopped being true.
   */
  async refresh() {
    try {
      const reply = await invoke<KitStateReply>('kit_state');
      const next = oneShotsOf(reply.lanes);
      set((state) => ({
        id: reply.id,
        lanes: reply.lanes,
        loaded: true,
        // ⛔ **The PREVIOUS object when nothing moved, and that is not a
        // micro-optimisation.** `history.changed` compares snapshot fields by
        // reference, and `refresh` runs after every gesture in the app — a
        // fresh object each time would report the kit as changed on every
        // snapshot, so one seed keystroke would record an undo step and
        // nothing would ever coalesce.
        oneShots: sameKit(state.oneShots, next) ? state.oneShots : next,
      }));
    } catch (error) {
      // ⛔ Reported, not swallowed. A panel that fails to load its own kit and
      // shows an empty grid is the readout-that-lies failure this store exists
      // to end, arriving through the error path instead.
      set({ error: reason(error), loaded: true });
    }
  },

  async assign(lane) {
    if (get().assigning) return;
    set({ assigning: lane, error: null });
    try {
      await invoke('one_shot_assign', { lane });
    } catch (error) {
      // ⛔ **A refusal because one is already open falls through to the poll**,
      // for the reason `runExport` gives: the plugin keeps one dialog slot and
      // only this poll drains it, so a page that stopped polling — a reloaded
      // webview, an editor torn down and reopened — would otherwise leave the
      // producer refused with no dialog anywhere on screen and no way back.
      if (!reason(error).includes('already')) {
        set({ assigning: null, error: reason(error) });
        return;
      }
    }

    // ⛔ **`awaitLoader`, not a second copy of it.** That function's own doc says
    // *"the poll is shared rather than written twice"* and this was the second
    // copy — same command, same single slot, same terminal handling, differing
    // only in clearing `assigning`. Two pollers over one mailbox is also a real
    // race rather than only duplication: `take_status` clears on read, so a
    // producer with the assign dialog open who pressed the dice had two loops
    // waiting on one answer and whichever read it first consumed it, leaving the
    // other to report success over the failure it never saw.
    //
    // ⚠ Plain sequence rather than a `try`/`finally`: `awaitLoader` catches its
    // own `invoke` failure and returns, and `refresh` catches its own, so there
    // is no rejection for a `finally` to be guarding against.
    await get().awaitLoader();
    set({ assigning: null });
    // ⛔ **One undo step, once the change has actually landed** (TASK-050A).
    // `noteDocumentChange` is the one door for a document that lives outside
    // the session store: it records AND saves, and it is a no-op when nothing
    // moved — a cancelled dialog leaves `oneShots` at the same object, so
    // `history.changed` sees no field change and records nothing.
    //
    // ⛔ **Here rather than in `refresh`, which every one of these ends with.**
    // A refresh also runs on mount and after an undo puts a kit back, and
    // recording there would put *"the kit finished loading"* on the stack — so
    // the producer's first Ctrl+Z would clear pads they never touched.
    noteDocumentChange();
  },

  async addMany() {
    if (get().busy()) return;
    // ⛔ **Claimed, not merely checked.** The first cut read `assigning` as a
    // guard and never set it, so the button's own `disabled` never engaged and
    // the guard could only ever see a dialog opened by `assign`. A dialog is
    // modal for as long as a producer browses a sample folder; a second one
    // pressed in that window is refused by the plugin and falls through to the
    // poll, which is a round trip to learn something the page already knew.
    //
    // ⛔ **Its own flag rather than parking a lane in `assigning`.** The second
    // cut set `assigning: 'kick'` as a stand-in for "a dialog is open", on the
    // claim that every reader only asks whether it is null. That was false:
    // `KitPanel` compares it to a lane to draw *"choosing…"* over that row's
    // sample name — so pressing **Add samples…** made the kick row stop naming
    // its own sample for as long as the dialog was open. A batch belongs to no
    // lane, so it does not get to borrow one.
    //
    // ⚠ **The last report goes as the new one starts**, for the reason
    // `refresh` gives about `error`: what the previous import did stopped being
    // true the moment this was pressed.
    set({ importing: true, error: null, imported: null });
    try {
      await invoke('one_shot_add_many');
    } catch (error) {
      // The same fall-through `assign` documents: only the poll drains the
      // plugin's one dialog slot, so a refusal for one already being open must
      // not skip it.
      if (!reason(error).includes('already')) {
        set({ importing: false, error: reason(error) });
        return;
      }
    }
    await get().awaitLoader();
    set({ importing: false });
    noteDocumentChange();
  },

  async loadSaved(id) {
    // ⛔ **The one door for a saved kit, for the reason `drop` is one.** This
    // ran in `SavedKits` as `invoke → awaitLoader` and recorded nothing, so the
    // producer's next Ctrl+Z — about a mute, a seed, anything — restored a
    // snapshot still naming the kit from *before* the load, and `restoreKit`
    // dutifully unloaded the kit they had just chosen.
    set({ error: null });
    try {
      await invoke('kits_load', { id });
    } catch (error) {
      set({ error: reason(error) });
      return;
    }
    await get().awaitLoader();
    noteDocumentChange();
  },

  busy() {
    // ⚠ **One question, two slots.** A per-lane assignment and a batch import
    // both hold the plugin's single dialog slot, and every control that has to
    // go quiet while one is open cares only that *something* does.
    return get().assigning !== null || get().importing;
  },

  async drop(lane, path, reversed = false) {
    // ⛔⛔ **The one door for a sample landing on a pad from the browser**, and
    // it exists because there are FIVE gestures that do it — the KIT row, two
    // on the pad grid, the explorer's own `→`/`←` keys, and the stage — each of
    // which called `useExplorer.dropOn` and then `refresh()` directly. Drag and
    // drop is the primary way a sample reaches a pad, so every one of them has
    // to record an undo step; five call sites each remembering to is how four
    // of them eventually do not.
    //
    // ⚠ **Returns whether it landed**, because three of the five open the pad
    // editor on success and must not open it over a refusal.
    const landed = await useExplorer.getState().dropOn(lane, path, reversed);
    await get().refresh();
    // ⚠ A refusal moved nothing, so `oneShots` is the same object and
    // `history.changed` records nothing — but asking is clearer than relying on
    // that, and it keeps a failed drop out of the `persist()` this triggers.
    if (landed) noteDocumentChange();
    return landed;
  },

  dismissImport() {
    set({ imported: null });
  },

  async clear(lane) {
    // Cleared as the action starts, for the reason `refresh` gives: whatever
    // went wrong last time stopped being true the moment this was pressed.
    set({ error: null });
    try {
      await invoke('one_shot_clear', { lane });
    } catch (error) {
      set({ error: reason(error) });
      return;
    }
    await get().refresh();
    noteDocumentChange();
  },

  async randomize(lane) {
    set({ error: null });

    const locked = new Set(useSession.getState().lockedLanes);
    const targets = (lane === null ? get().lanes.map((entry) => entry.lane) : [lane]).filter(
      (candidate) => !locked.has(candidate),
    );

    if (targets.length === 0) {
      // ⚠ **Said rather than silently doing nothing** — a dice that appears to
      // work and changes no sound is the readout-that-lies failure in
      // miniature. ⛔ And it says which case it is: one locked pad reported
      // "every pad is locked", which is a claim about the whole kit that is
      // simply false, and left a producer looking for a lock they had not set.
      const all = get().lanes.length === 0 ? 'the kit has not loaded yet' : null;
      set({
        error: reason(
          new Error(all ?? (lane === null ? 'every pad is locked' : `${lane} is locked`)),
        ),
      });
      return;
    }

    try {
      // ⚠ **The seed is taken here**, the same rule the variation log follows:
      // nothing below the page may read a clock, and a re-roll still has to be
      // a different roll each time.
      // ⛔ **The folder the browser is standing in travels with the request** —
      // `standingIn` carries Mike's rule: a selected *file* means its folder, a
      // folder means itself. The plugin used to answer this from its own current
      // folder, which the tree view stopped maintaining.
      await invoke('kit_randomize', {
        lanes: targets,
        seed: String(Date.now()),
        folder: standingIn(useExplorer.getState()),
      });
    } catch (error) {
      set({ error: reason(error) });
      return;
    }
    await get().awaitLoader();
    noteDocumentChange();
  },

  editPad(lane) {
    set({ editingPad: lane });
    // ⚠ Only on the way *in*: closing an editor should not rearrange the rail
    // the producer is looking at.
    if (lane !== null) useUi.getState().showSection('kit');
  },

  async setTweaks(lane, patch) {
    const before = get().lanes;
    const row = before.find((entry) => entry.lane === lane);
    if (row === undefined) return;

    const tweaks = { ...row.tweaks, ...patch };
    set({
      error: null,
      lanes: before.map((entry) => (entry.lane === lane ? { ...entry, tweaks } : entry)),
    });

    try {
      await invoke('pad_tweaks_set', { lane, tweaks });
    } catch (error) {
      // ⛔ **Put the row back.** A control left showing a value the plugin
      // refused is the readout-that-lies failure this store was written to end,
      // and it is worse on a knob than on a label: the producer goes on turning
      // it, hearing nothing change, with no reason on screen.
      set({ lanes: before, error: reason(error) });
    }
  },

  async awaitLoader() {
    // ⛔⛔ **Without this the panel refreshed onto stale state and a failure was
    // never shown at all.** `randomize` and `load_kit` hand the decode to the
    // loader thread and answer immediately, exactly as `assign` does — so a
    // resolved promise means "started", not "done". Refreshing straight after
    // read the kit mid-decode, and a saved kit whose files had moved published
    // `Failed` into a slot nothing ever read: the producer saw no error and
    // unchanged pads. That is the readout-that-lies failure this codebase keeps
    // recording, so the poll is shared rather than written twice.
    //
    // ⛔⛔ **SINGLE-FLIGHT, and deleting the second *copy* of this loop was not
    // enough on its own.** There are four callers — `assign`, `randomize`,
    // `SavedKits` and the project restore — and `assign`'s own `if
    // (get().assigning) return` guards only re-entry into `assign`. So pressing
    // Assign and then the dice put two loops on `one_shot_status`, which
    // `take_status` **clears on read**: whichever polled first consumed the
    // answer and the other reported success over a failure it never saw. One
    // reader per mailbox, by construction — a second caller waits on the first
    // one's promise instead of opening its own poll.
    const inFlight = loading;
    if (inFlight !== null) return inFlight;

    const run = (async () => {
      for (;;) {
        let status: OneShotStatus;
        try {
          status = await invoke<OneShotStatus>('one_shot_status');
        } catch (error) {
          set({ error: reason(error) });
          return;
        }
        if (status.state !== 'running') {
          // ⚠ Cancelled is **not** an error. Closing the dialog is the ordinary
          // way out of it, and reporting it would train people to ignore the one
          // message that matters.
          //
          // ⚠ **Nor is a batch that refused some of its files** (TASK-049). It
          // lands in `imported`, which the panel draws as a report rather than
          // an alert — see that field for why the two are kept apart.
          //
          // ⚠ **A restore says nothing unless something could not come back**
          // (TASK-050A). An undo is machine driven, not a dialog the producer
          // is standing in front of, so a clean one is silent — the same rule
          // `oneshot::restore` states for a reopened project. A sample that has
          // moved since is still reported, because a pad that did not come back
          // is something they have to know.
          set({
            error:
              status.state === 'failed'
                ? status.reason
                : status.state === 'restored' && status.refused.length > 0
                  ? // ⛔ **Every one of them, not the first.** An undo that
                    // crosses three samples that have since moved reverts three
                    // pads to their shipped sounds; naming one of them and
                    // dropping the other two is the same *"a sample that did
                    // not come back is something the producer has to know"*
                    // rule applied to a third of the evidence.
                    status.refused.map((one) => `${one.name} — ${one.reason}`).join('; ')
                  : null,
            imported:
              status.state === 'imported'
                ? { loaded: status.loaded, refused: status.refused }
                : null,
          });
          // The kit only changed if something actually loaded, but re-reading is
          // cheap and it is the one call that cannot get the panel out of step.
          await get().refresh();
          return;
        }
        // ⚠ No ceiling: the loader thread always publishes a terminal status, so
        // `running` genuinely means a dialog is open — however long somebody
        // spends finding a kick. The export poll has none for the same reason.
        await new Promise((resume) => setTimeout(resume, ONE_SHOT_POLL_MS));
      }
    })();

    // ⚠ **Cleared however it ends**, or one thrown poll would wedge every later
    // caller onto a promise that has already settled — the mirror of the
    // `Release` guard `extract::job` uses on the other side of the bridge.
    loading = run.finally(() => {
      loading = null;
    });
    return loading;
  },
}));

/**
 * Hand the session store the kit, so one Ctrl+Z covers it (TASK-050A).
 *
 * ⛔ **Registered here rather than imported there, because `session.ts` cannot
 * import this file** — this one imports it, and `snapshotOf` is synchronous so
 * the `await import('./kit')` trick that breaks the cycle elsewhere is not
 * available to it. The arrangement inverts the same dependency the same way;
 * see `registerSongDocument`.
 */
registerKitDocument(() => ({ oneShots: useKit.getState().oneShots }), restoreKit);

/**
 * The kit an in-flight restore should end at, or `null` when none is wanted.
 *
 * ⚠ Module state rather than a store field, for the reason `loading` above is:
 * nothing renders from it, and a `Promise` in the undo snapshot's neighbourhood
 * would re-render every subscriber for something no component draws.
 */
let wanted: AssignedKit | null = null;
let restoring: Promise<void> | null = null;

/**
 * Put a whole kit back, as an undo or a redo step (TASK-050A).
 *
 * ⛔⛔ **Compared before it is sent, and that guard is what makes the feature
 * affordable.** Every undo step carries the kit, so without it, stepping back
 * one seed keystroke would ask the plugin to re-decode a producer's twelve
 * samples — cutting every sounding voice — to arrive at the kit already loaded.
 *
 * ⛔⛔ **A restore already running keeps the slot, and the newest wanted kit is
 * PARKED rather than sent.** Restoring is asynchronous — an invoke, then the
 * loader poll — while the session fields it travels with are restored
 * synchronously. Ctrl+Z held down therefore issued a second `one_shot_set_all`
 * while the first was still decoding, and the plugin keeps **one** loader slot:
 * the second was refused with *"already"* and surfaced as an error over a kit
 * left a step behind the rest of the undo. Last-write-wins is the right rule,
 * because the last snapshot applied is the one the producer is looking at.
 *
 * ⚠ **A named export rather than a closure inside the registration**, so this
 * can be driven directly by a test — the seam is installed once at module load
 * and replacing it to observe it would remove the thing under test.
 *
 * Returns when nothing is left to restore, so a test can await it.
 */
export function restoreKit({ oneShots }: { oneShots: AssignedKit }): Promise<void> {
  // ⛔⛔ **Compared against where the kit is HEADING, not where it is.** While a
  // restore is in flight the store still holds the kit it is leaving, so asking
  // the store alone would answer about a state already being replaced: undo to
  // B and immediately redo to A, and *"A is what is loaded"* is true and
  // useless — B is what is arriving. Reading `wanted` first is what lets the
  // redo cancel the undo instead of landing under it.
  const heading = wanted ?? useKit.getState().oneShots;
  if (sameKit(heading, oneShots)) return restoring ?? Promise.resolve();
  wanted = oneShots;
  if (restoring !== null) return restoring;

  const run = (async () => {
    // ⚠ **`wanted` is cleared only once its restore has finished**, and only if
    // nothing newer arrived meanwhile — that is what keeps the check above
    // truthful for the whole time a restore is running.
    while (wanted !== null) {
      const next: AssignedKit = wanted;
      if (sameKit(useKit.getState().oneShots, next)) {
        if (wanted === next) wanted = null;
        continue;
      }

      // ⚠ **Sent as triples, in the shape `one_shot_set_all` parses** — lane,
      // path, and which way round. A lane the snapshot does not mention goes
      // back to its shipped sound, which is what makes undoing an assignment
      // reachable at all.
      const lanes = Object.entries(next).map(
        ([lane, one]: [string, AssignedKit[string]]) => [lane, one.path, one.reversed] as const,
      );
      try {
        await invoke('one_shot_set_all', { lanes });
        await useKit.getState().awaitLoader();
      } catch (error) {
        useKit.setState({ error: reason(error) });
        wanted = null;
        return;
      }
      if (wanted === next) wanted = null;
    }
  })();

  // ⚠ **Cleared however it ends**, or one thrown restore would wedge every
  // later one behind a latch nothing will ever release — the same rule
  // `awaitLoader`'s own `finally` follows.
  restoring = run.finally(() => {
    restoring = null;
  });
  return restoring;
}
