/**
 * Dragging a generated part out of the plugin and into the DAW (TASK-063C).
 *
 * Mike, 2026-08-05: *"you need to be able to drag each generator's midi or
 * audio from the generator to the DAW"* … *"same with the song arrangement"*.
 *
 * ## ⛔ Nothing here is an HTML5 drag, and that is the whole point
 *
 * A `dragstart` inside a webview is handed to the *page*, not to the window
 * server, so the DAW never sees a file. The real drag is started natively by
 * `plugin/src/drag` — this store's job is to run the *gesture* on this side and
 * tell the plugin when to take over. So: no `draggable`, no `dataTransfer`, no
 * `setDragImage`. A pointer press, a threshold, and three commands.
 *
 * ## The gesture, and the race inside it
 *
 * Rendering four bars of audio takes long enough that the drag cannot start on
 * the same tick as the press, so the sequence is:
 *
 * 1. `pointerdown` → `drag_prepare` starts rendering on a plugin thread.
 * 2. `pointermove` past {@link THRESHOLD_PX} → the producer means it, and the
 *    drag image is drawn.
 * 3. When *both* have happened → `drag_start`, which blocks until they let go.
 *
 * ⛔ **The producer can release before the bytes are ready, and that is the case
 * to get right.** Starting the OS drag then would make `DoDragDrop` see no
 * button held on its first question, answer "dropped", and put the files
 * wherever the cursor was sitting — a loop landing on a random track. So a
 * release before the drag begins sends `drag_cancel` instead, and the plugin
 * refuses a second time on its own by checking the button state natively.
 *
 * ⚠ **The gesture is owned by the chip that started it**, and passed in as a
 * {@link Gesture}. It used to be two module-level booleans, which meant a second
 * chip pressed mid-gesture reset the first one's state — reachable with touch,
 * which the chips deliberately enable — and left `begin` with an ordering
 * contract nothing could enforce.
 */

import { create } from 'zustand';

import { invoke } from '../lib/ipc';
import { reason } from './session';
import type { PreviewPayload } from '../components/Kit/dragPreview';
import type { Lane, Pattern, Song } from '../lib/ipc-types';

/** How far the pointer moves before a press counts as a drag rather than a click. */
export const THRESHOLD_PX = 6;

/** How often the page asks whether the files are written. */
export const DRAG_POLL_MS = 40;

/**
 * How long to wait **without progress** before giving up.
 *
 * ⛔⛔ **A stall timeout, not a total one, and that distinction is what lets a
 * whole arrangement be dragged as audio.** It used to be a flat ceiling on the
 * gesture, reasoning that "a render that has not finished in ten seconds is not
 * going to become a drag anybody still has their finger down for". That was
 * true while the longest render was a four-bar loop. A three-minute record is
 * genuinely seconds of work, and killing it at ten would make the feature
 * impossible rather than protecting anybody from it.
 *
 * What the ceiling was actually for is a render that has *stopped* — a thread
 * that died, a status that never moves. That is what this measures now: the
 * clock restarts every time the plugin reports it has got further.
 */
export const DRAG_STALL_MS = 10_000;

/** What `drag_status` answers with. Mirrors `drag::Status` in the plugin. */
type DragStatus =
  | { state: 'idle' }
  /** `done` is 0.0–1.0. Absent from a plugin built before it existed. */
  | { state: 'preparing'; done?: number }
  | { state: 'ready' }
  | { state: 'failed'; reason: string };

export type DragFormat = 'midi' | 'audio';

/**
 * What is being picked up.
 *
 * ⚠ **A lane is *named*, not sliced.** The page used to build a one-lane copy of
 * the pattern and ask for "split by lane", which put "what a lane stem is" on
 * both sides of the bridge; `export::Cut` owns that now.
 */
export type DragSubject =
  | { kind: 'patterns'; patterns: Pattern[]; lanes?: boolean; lane?: Lane }
  | { kind: 'song'; song: Song };

/**
 * The live state of one press, owned by the chip it started on.
 *
 * Mutable and outside React on purpose: it changes on raw pointer events and
 * must not cost a render. A ref rather than a module global so two chips cannot
 * share one.
 */
export type Gesture = {
  /** False once the producer lets go. */
  held: boolean;
  /** True once the pointer has travelled past the threshold. */
  moved: boolean;
  /** Drawn at the threshold, sent with `drag_start`. */
  preview: PreviewPayload | null;
};

type DragState = {
  /**
   * Whether this build has a native drag source at all.
   *
   * ⛔ **Answered by the plugin, not inferred from the platform.** macOS and
   * Linux have none yet, and a handle that looks draggable and drops nothing is
   * worse than no handle — the producer blames their DAW.
   */
  canDrag: boolean;
  /** Set once from `app_info` at launch. */
  loadCapability: () => Promise<void>;

  state: 'idle' | 'preparing' | 'dragging' | 'failed';
  message: string | null;
  /**
   * How far the render has got, 0.0–1.0, while `state` is `preparing`.
   *
   * ⚠ **Stays 0 for the short renders**, which is most of them: a four-bar loop
   * finishes inside one poll, so a bar that flashed on and off would be noise.
   * The page shows a plain "getting it ready" until this passes zero — see
   * `drag::Status::Preparing` on the plugin side, which makes the same call.
   */
  progress: number;

  /** Begin the gesture. Resolves when the drag has ended, one way or another. */
  begin: (subject: DragSubject, format: DragFormat, gesture: Gesture) => Promise<void>;
  /** The producer let go before it became a drag. */
  abandon: () => void;
};

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));

export const useDrag = create<DragState>((set, get) => ({
  canDrag: false,
  state: 'idle',
  message: null,
  progress: 0,

  async loadCapability() {
    try {
      const { supported } = await invoke<{ supported: boolean }>('drag_supported');
      set({ canDrag: supported === true });
    } catch {
      // ⚠ Silent, and it degrades to "no drag". What this must not do is light
      // up a handle on the strength of not knowing.
      set({ canDrag: false });
    }
  },

  async begin(subject, format, gesture) {
    // ⛔ **`failed` is a state a fresh press must clear, not one it bounces
    // off.** Refusing every state but `idle` meant the first press after any
    // failure did nothing at all — no render, no message, and the release then
    // wiped the error too, so only the *second* press worked and the producer
    // never learned why the first did not.
    if (!get().canDrag || get().state === 'preparing' || get().state === 'dragging') return;
    // ⚠ Zeroed with the state, not left from the last gesture — a bar that
    // opened at the previous drag's 80% and then jumped backwards would be
    // worse than no bar.
    set({ state: 'preparing', message: null, progress: 0 });

    try {
      await invoke('drag_prepare', {
        patterns: subject.kind === 'patterns' ? subject.patterns : [],
        song: subject.kind === 'song' ? subject.song : null,
        audio: format === 'audio',
        lanes: subject.kind === 'patterns' ? (subject.lanes ?? false) : false,
        lane: subject.kind === 'patterns' ? (subject.lane ?? null) : null,
      });
    } catch (error) {
      set({ state: 'failed', message: reason(error) });
      return;
    }

    // ⛔ Pushed forward every time the render reports it has got further, so a
    // long render is bounded by *stalling* rather than by taking a long time.
    let until = Date.now() + DRAG_STALL_MS;
    // ⚠ Cached, so the poll stops once the answer can no longer change. Without
    // this a producer who presses and holds without moving sends a round trip
    // every 40 ms — 250 of them — to the thread the DAW draws its editor from,
    // each one re-learning a fact this side already knew.
    let ready = false;
    for (;;) {
      // ⛔ Checked before the status, not after: a producer who has already let
      // go must never reach `drag_start`, however ready the files are.
      if (!gesture.held) {
        await invoke('drag_cancel').catch(() => undefined);
        set({ state: 'idle' });
        return;
      }
      if (Date.now() > until) {
        await invoke('drag_cancel').catch(() => undefined);
        set({ state: 'failed', message: 'that stopped making progress' });
        return;
      }

      if (!ready) {
        let status: DragStatus;
        try {
          status = await invoke<DragStatus>('drag_status');
        } catch (error) {
          set({ state: 'failed', message: reason(error) });
          return;
        }
        if (status.state === 'failed') {
          set({ state: 'failed', message: status.reason });
          return;
        }
        // ⚠ `idle` means the plugin disowned this render — a newer press took
        // the slot. Polling on would only run to the timeout and report a
        // failure for a drag that was superseded on purpose.
        if (status.state === 'idle') {
          set({ state: 'idle' });
          return;
        }
        // ⛔ **Any forward movement re-arms the stall clock.** Without this the
        // ceiling would still kill a whole-arrangement render at ten seconds,
        // which is the limit `DRAG_STALL_MS` exists to stop being.
        if (status.state === 'preparing') {
          const done = typeof status.done === 'number' ? status.done : 0;
          // ⚠ Compared against the store rather than a local copy of it. The
          // two were the same number by construction, and two variables that
          // have to stay in lockstep are two that can stop — the failure there
          // would be a stall clock that never re-arms, which is silent and only
          // shows up on the long renders this exists for.
          //
          // ⛔ **Any CHANGE re-arms it, not any increase.** A render that is
          // reporting at all is alive, and the figure can legitimately go
          // *down*: every `drag_prepare` zeroes the plugin's counter, so a
          // second press that follows an abandoned one at 80% reports 0.0 and
          // then climbs. Under `>` none of that counted as progress, the clock
          // never re-armed, and ten seconds later the page cancelled a healthy
          // render and told the producer "that stopped making progress".
          if (done !== get().progress) {
            until = Date.now() + DRAG_STALL_MS;
            set({ progress: done });
          }
        }
        ready = status.state === 'ready';
      }
      // ⛔⛔ **Re-tested here, after the await, and that is the whole point.**
      // The check at the top of the loop reads `held` *before* a round trip to
      // the editor thread — which is also serving the host poll and the export
      // poll, so it is not instant. A producer who let go during that await
      // would otherwise fall straight through to `drag_start`, which is exactly
      // the path this file's own comment claims is impossible. The plugin's
      // `button_is_down()` is the second line of defence, not the first, and it
      // reads `GetAsyncKeyState(VK_LBUTTON)` — which is not set for a touch or
      // pen pointer at all.
      if (ready && gesture.moved && gesture.held) {
        set({ state: 'dragging' });
        break;
      }
      // ⚠ `ready` without the threshold is not an exit: the files are written
      // and the producer has simply not moved yet. Leaving the loop here is what
      // would turn a click into a drag.
      await sleep(DRAG_POLL_MS);
    }

    try {
      // Blocks for the whole gesture — the plugin is inside the OS drag loop.
      await invoke<'copied' | 'cancelled'>('drag_start', { preview: gesture.preview });
      set({ state: 'idle' });
    } catch (error) {
      set({ state: 'failed', message: reason(error) });
    }
  },

  abandon() {
    // ⚠ `preparing` is left to `begin`'s own loop, which is about to see the
    // gesture is no longer held and cancel it. Cancelling here as well would
    // race that.
    //
    // ⛔ **`failed` is left alone too, and that is the fix for a message nobody
    // could read.** The error is rendered only while the state is `failed`, and
    // a release follows the press by a couple of hundred milliseconds — so
    // resetting to `idle` here erased "the preview kit did not load" before the
    // producer's eyes reached it. The next press clears it instead, which is
    // the moment they have stopped reading.
    if (get().state !== 'dragging') return;
    void invoke('drag_cancel').catch(() => undefined);
    set({ state: 'idle' });
  },
}));
