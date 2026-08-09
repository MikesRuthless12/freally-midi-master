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
import { reason } from './session';
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

type KitState = {
  /** The loaded kit's id, or `null` before the first read / if none loaded. */
  id: string | null;
  lanes: KitLane[];
  /** Whether `kit_state` has ever answered. Distinguishes "empty" from "not asked". */
  loaded: boolean;
  /** The lane whose dialog is open, or `null`. */
  assigning: Lane | null;
  /** The last thing that went wrong, for the panel to show. */
  error: string | null;
  refresh: () => Promise<void>;
  assign: (lane: Lane) => Promise<void>;
  clear: (lane: Lane) => Promise<void>;
};

export const useKit = create<KitState>((set, get) => ({
  id: null,
  lanes: [],
  loaded: false,
  assigning: null,
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

    const tick = async (): Promise<void> => {
      let status: OneShotStatus;
      try {
        status = await invoke<OneShotStatus>('one_shot_status');
      } catch (error) {
        set({ assigning: null, error: reason(error) });
        return;
      }
      if (status.state === 'running') {
        // No ceiling, for the reason the export poll has none: the loader
        // thread always publishes a terminal status, so `running` genuinely
        // means a dialog is open — however long somebody spends finding a kick.
        setTimeout(() => void tick(), ONE_SHOT_POLL_MS);
        return;
      }
      set({
        assigning: null,
        // ⚠ Cancelled is **not** an error. Closing the dialog is the ordinary
        // way out of it, and reporting it would train people to ignore the one
        // message that matters.
        error: status.state === 'failed' ? status.reason : null,
      });
      // The kit only changed if something actually loaded, but re-reading is
      // cheap and it is the one call that cannot get the panel out of step.
      await get().refresh();
    };
    await tick();
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
}));
