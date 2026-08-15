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
import { reason, useSession } from './session';
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
};

type KitStateReply = {
  id: string | null;
  lanes: KitLane[];
};

/** What `one_shot_status` answers with. Mirrors `oneshot::Status`. */
type OneShotStatus =
  | { state: 'idle' }
  | { state: 'running' }
  | { state: 'done'; lane: Lane; name: string }
  | { state: 'cancelled' }
  | { state: 'failed'; reason: string };

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

type KitState = {
  /** The loaded kit's id, or `null` before the first read / if none loaded. */
  id: string | null;
  lanes: KitLane[];
  /** Whether `kit_state` has ever answered. Distinguishes "empty" from "not asked". */
  loaded: boolean;
  /** The lane whose dialog is open, or `null`. */
  assigning: Lane | null;
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
  refresh: () => Promise<void>;
  assign: (lane: Lane) => Promise<void>;
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
  editingPad: null,
  error: null,

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
      set({ id: reply.id, lanes: reply.lanes, loaded: true });
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
          set({ error: status.state === 'failed' ? status.reason : null });
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
