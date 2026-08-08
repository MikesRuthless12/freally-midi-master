/**
 * The sample browser and its audition player (TASK-132).
 *
 * ⛔ **The Rust for all of this shipped with no caller.** `plugin/src/explorer.rs`
 * and `plugin/src/preview.rs` were written, tested and dead: saved library
 * folders, the containment boundary, the waveform peaks and an audition voice
 * with seek, reverse, loop and a published position — and nothing in `src/`
 * invoked a single command. This store is that caller.
 *
 * ⛔ **Six of Mike's eight preview items are one number.** The playhead marker,
 * the progress fill, the time readout, click-to-seek, reverse and loop all
 * resolve to the read position, which `preview_position` publishes the way the
 * pattern playhead is published. Everything the panel draws follows from
 * [`PreviewPosition`]; building six channels would have been six things to keep
 * in step.
 */

import { create } from 'zustand';

import { invoke } from '../lib/ipc';
import { isPlugin } from '../lib/ipc-plugin';
import { readStored, writeStored } from './storage';
import { reason } from './session';
import type { Lane } from '../lib/ipc-types';

/** One row in the browser. Mirrors `explorer::Entry`. */
export type ExplorerEntry = {
  name: string;
  path: string;
  isDir: boolean;
};

/** What `explorer_state` answers with. Mirrors `explorer::State`. */
type ExplorerReply = {
  roots: ExplorerEntry[];
  folder: string | null;
  parent: string | null;
  entries: ExplorerEntry[];
  truncated: boolean;
  /** Whether a folder dialog is open right now — see `addFolder`. */
  picking: boolean;
};

/** What `explorer_waveform` answers with. Mirrors `explorer::Waveform`. */
export type Waveform = {
  path: string;
  name: string;
  /** `[min, max]` per column, each in -1..=1. Both bounds, never one amplitude. */
  peaks: [number, number][];
  seconds: number;
};

/** What `preview_position` answers with. Mirrors `preview::Position`. */
export type PreviewPosition = {
  playing: boolean;
  seconds: number;
  total: number;
  looping: boolean;
  reverse: boolean;
};

const STOPPED: PreviewPosition = {
  playing: false,
  seconds: 0,
  total: 0,
  looping: false,
  reverse: false,
};

/**
 * How often the position is read while a sample is sounding.
 *
 * 30 Hz, the same rate `subscribeToPlayhead` documents and for the same reason:
 * the marker is one number against an atomic the audio thread already writes
 * every block, and rAF would double the round trips to move a line across a
 * panel a few hundred pixels wide.
 */
const PLAYING_POLL_MS = 33;

/**
 * ...and while one is loaded but stopped.
 *
 * ⛔ **Not zero, and that is not politeness.** `preview_position` is also what
 * calls `Preview::collect` — the editor-thread half of the buffer handoff, which
 * frees the `Vec` the audio callback parked rather than dropping it on the
 * callback. Stop polling entirely when paused and the previous sample's memory
 * is held until the next time something plays.
 */
const IDLE_POLL_MS = 500;

const WIDTH_KEY = 'freally.browserWidth';

/**
 * How wide the browser rail may be dragged.
 *
 * ⛔ **Mike asked for the ceiling by name**, 2026-08-07: *"don't let it get
 * absurdly wide."* The floor is not a taste call either — dragged to nothing the
 * handle has no target left to grab, so the panel could be closed and never
 * reopened by the same gesture that closed it.
 */
export const RAIL_MIN_WIDTH = 240;
export const RAIL_MAX_WIDTH = 560;
export const RAIL_DEFAULT_WIDTH = 280;

export function clampRailWidth(px: number): number {
  if (!Number.isFinite(px)) return RAIL_DEFAULT_WIDTH;
  return Math.min(RAIL_MAX_WIDTH, Math.max(RAIL_MIN_WIDTH, Math.round(px)));
}

function loadRailWidth(): number {
  const raw = readStored(WIDTH_KEY, (v): v is string => typeof v === 'string', '');
  // ⚠ No NaN branch: `clampRailWidth` already answers the default for a value
  // that is not finite, and a second copy of that rule is one more to revisit.
  return clampRailWidth(Number.parseInt(raw, 10));
}

type ExplorerStore = {
  roots: ExplorerEntry[];
  folder: string | null;
  parent: string | null;
  entries: ExplorerEntry[];
  truncated: boolean;
  /** Whether a folder dialog is open — drives the poll in `addFolder`. */
  picking: boolean;
  /** Whether `explorer_state` has ever answered — "empty" is not "not asked". */
  loaded: boolean;
  /** The file being auditioned, or `null`. */
  selected: string | null;
  waveform: Waveform | null;
  position: PreviewPosition;
  error: string | null;
  /** The left rail's width in CSS pixels, persisted. */
  railWidth: number;

  refresh: () => Promise<void>;
  addFolder: () => Promise<void>;
  removeFolder: (path: string) => Promise<void>;
  open: (path: string) => Promise<void>;
  select: (path: string) => Promise<void>;
  play: () => Promise<void>;
  pause: () => Promise<void>;
  stop: () => Promise<void>;
  seek: (seconds: number) => Promise<void>;
  toggleLoop: () => Promise<void>;
  setReverse: (on: boolean) => Promise<void>;
  /** Put the selected sample on a drum lane. */
  dropOn: (lane: Lane, path: string) => Promise<void>;
  setRailWidth: (px: number) => void;
};

export const useExplorer = create<ExplorerStore>((set, get) => ({
  roots: [],
  folder: null,
  parent: null,
  entries: [],
  truncated: false,
  picking: false,
  loaded: false,
  selected: null,
  waveform: null,
  position: STOPPED,
  error: null,
  railWidth: loadRailWidth(),

  async refresh() {
    try {
      const reply = await invoke<ExplorerReply>('explorer_state');
      set({
        roots: reply.roots,
        folder: reply.folder,
        parent: reply.parent,
        entries: reply.entries,
        truncated: reply.truncated,
        picking: reply.picking,
        loaded: true,
      });
    } catch (error) {
      // Reported rather than swallowed, for the reason `useKit.refresh` gives:
      // a browser that fails to read its own library and draws an empty list is
      // the readout-that-lies failure, arriving through the error path.
      set({ error: reason(error), loaded: true });
    }
  },

  async addFolder() {
    set({ error: null });
    try {
      await invoke('explorer_pick');
    } catch (error) {
      // ⚠ **A refusal because one is already open falls through to the poll**,
      // exactly as `useKit.assign` does: the dialog runs on its own thread and
      // only a later `explorer_state` can learn that it finished.
      if (!reason(error).includes('already')) {
        set({ error: reason(error) });
        return;
      }
    }
    // ⛔ **Polled, because the dialog thread cannot tell the page anything.** It
    // is modal on a thread of its own — see `Explorer::pick` — so the only way a
    // newly added root becomes visible is by asking again.
    //
    // ⛔⛔ **The exit is the plugin's own `picking` flag, not a counter.** This
    // used to run a fixed 301 attempts at 400 ms whatever happened, so
    // *cancelling* the dialog cost two minutes of polling — and each poll is a
    // `read_dir` over a folder that may hold 2,000 entries, serialized across
    // the bridge and re-rendered as 2,000 rows. `Explorer::pick` already tracks
    // whether a dialog is open and clears it on every path including a cancel;
    // the poll now lasts exactly as long as the dialog does.
    //
    // ⚠ **`await`ed to completion**, so the caller's promise means "the dialog
    // is finished" rather than "the first poll is". A fire-and-forget chain here
    // also had no way to be cancelled.
    // ⚠ The cap stays as a backstop for a `picking` that somehow never clears —
    // ten minutes, past any real interaction with a folder picker.
    for (let attempt = 0; attempt < 1_500; attempt += 1) {
      await get().refresh();
      if (!get().picking) return;
      await new Promise((done) => window.setTimeout(done, 400));
    }
  },

  async removeFolder(path) {
    set({ error: null });
    try {
      await invoke('explorer_remove', { path });
    } catch (error) {
      set({ error: reason(error) });
      return;
    }
    await get().refresh();
    // ⛔⛔ **Asked of the plugin rather than answered by comparing strings.**
    // This used to be `selected.startsWith(path)`, which never matched on
    // Windows: `Explorer::open` stores the *canonical* browse location, so the
    // entries it lists come back as `\\?\C:\…` while a root is kept exactly as
    // it was added (`C:\…`). Removing the folder you were browsing left the
    // preview player still holding — and still drawing — a sample the browser
    // could no longer reach. `same_or_inside`'s own doc records this canonical
    // -versus-raw comparison causing two earlier bugs on this very path.
    //
    // ⚠ The plugin already clears its browse location when the root it was
    // inside goes, so a `null` folder after the refresh *is* the answer, in the
    // one spelling both sides agree on. Removing some *other* root leaves the
    // folder set and the selection alone, which is right.
    if (get().folder === null) {
      set({ selected: null, waveform: null });
    }
  },

  async open(path) {
    set({ error: null });
    try {
      await invoke('explorer_open', { path });
    } catch (error) {
      set({ error: reason(error) });
      return;
    }
    await get().refresh();
  },

  async select(path) {
    // ⛔ **Set before either call, so the panel switches immediately** and a
    // slow decode cannot leave the previous sample's waveform on screen looking
    // like the one that was just clicked.
    set({ selected: path, waveform: null, error: null });
    try {
      const [wave] = await Promise.all([
        invoke<Waveform>('explorer_waveform', { path }),
        invoke('preview_load', { path }),
      ]);
      // ⛔ **A late reply must not draw over a newer one.** Clicking down a
      // folder faster than the decodes come back would otherwise leave whichever
      // finished last on screen, which is not necessarily the one selected. The
      // waveform carries its own path precisely so this check is possible.
      if (get().selected !== wave.path) return;
      set({ waveform: wave });
    } catch (error) {
      if (get().selected !== path) return;
      set({ error: reason(error), waveform: null });
    }
  },

  async play() {
    try {
      await invoke('preview_play');
      poke();
      set({ position: { ...get().position, playing: true } });
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  async pause() {
    try {
      await invoke('preview_pause');
      poke();
      set({ position: { ...get().position, playing: false } });
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  // ⛔ Rewinds to the start. Pause holds position; stop does not — Mike named
  // both in the same sentence, which is why they are two commands.
  async stop() {
    try {
      await invoke('preview_stop');
      poke();
      set({ position: { ...get().position, playing: false, seconds: 0 } });
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  async seek(seconds) {
    // Written through immediately as well as sent, so the playhead lands under
    // the cursor on the frame of the click rather than on the next poll.
    set({ position: { ...get().position, seconds } });
    try {
      await invoke('preview_seek', { seconds });
      poke();
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  async toggleLoop() {
    const on = !get().position.looping;
    set({ position: { ...get().position, looping: on } });
    try {
      await invoke('preview_loop', { on });
      poke();
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  async setReverse(on) {
    set({ position: { ...get().position, reverse: on } });
    try {
      await invoke('preview_reverse', { on });
      poke();
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  async dropOn(lane, path) {
    set({ error: null });
    try {
      await invoke('explorer_drop', { lane, path });
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  setRailWidth(px) {
    const railWidth = clampRailWidth(px);
    writeStored(WIDTH_KEY, String(railWidth));
    set({ railWidth });
  },
}));

/**
 * Follow the audition position the audio thread publishes.
 *
 * The same shape as `subscribeToPlayhead`, and for the same reason it gives: the
 * plugin's bridge is a request/response custom protocol, so there is nothing to
 * push a position and the page asks. Two cadences rather than one because an
 * idle poll still has to run — see [`IDLE_POLL_MS`].
 */
export function subscribeToPreview(): () => void {
  if (!isPlugin()) return () => {};

  let live = true;
  const tick = async () => {
    if (!live) return;
    // Nothing loaded: nothing to read, and nothing parked to free either.
    if (useExplorer.getState().selected === null) {
      schedule(IDLE_POLL_MS);
      return;
    }
    try {
      const position = await invoke<PreviewPosition>('preview_position');
      const current = useExplorer.getState().position;
      // ⛔ Only when it moved. `set` on every frame re-renders the waveform
      // whether or not the playhead went anywhere, and the marker is a CSS
      // variable precisely so it does not have to.
      if (
        current.playing !== position.playing ||
        current.seconds !== position.seconds ||
        current.total !== position.total ||
        current.looping !== position.looping ||
        current.reverse !== position.reverse
      ) {
        useExplorer.setState({ position });
      }
      schedule(position.playing ? PLAYING_POLL_MS : IDLE_POLL_MS);
      return;
    } catch {
      // A dropped poll is a dropped frame of the marker and the next one fixes
      // it. Putting an error on screen for that would be noise.
    }
    schedule(IDLE_POLL_MS);
  };

  // ⛔ **One timer, replaced rather than added to.** `poke` reschedules, so a
  // second one running alongside the first would double the poll rate for the
  // life of the editor.
  let timer: number | null = null;
  const schedule = (ms: number) => {
    if (!live) return;
    if (timer !== null) window.clearTimeout(timer);
    timer = window.setTimeout(() => void tick(), ms);
  };

  // ⛔⛔ **Pressing Play could leave the marker sitting still for half a
  // second.** The idle cadence is 500 ms, and a transport command issued in the
  // middle of one of those waits did not shorten it — so the sample started
  // sounding immediately and the playhead did not move until the sleep expired.
  // On a one-shot shorter than 500 ms the whole audition could finish before the
  // marker moved at all, which reads as the player being broken.
  wake = () => schedule(0);

  void tick();
  return () => {
    live = false;
    if (timer !== null) window.clearTimeout(timer);
    wake = null;
  };
}

/**
 * Ask the position poll to run now rather than after its next wait.
 *
 * ⚠ **Module-level and nullable**, because there is exactly one subscription per
 * page and it does not exist outside the plugin — the same shape
 * `subscribeToPlayhead`'s own `live` flag uses. Calling it when nothing is
 * subscribed is a no-op rather than an error, which is what makes it safe for
 * the transport actions to call unconditionally.
 */
let wake: (() => void) | null = null;

function poke(): void {
  wake?.();
}

/** `83.4` → `1:23.4`. The readout is "playback time out of total time". */
export function formatSeconds(seconds: number): string {
  const safe = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
  const minutes = Math.floor(safe / 60);
  const rest = safe - minutes * 60;
  // One decimal, because a one-shot is often under a second and a whole-second
  // readout would sit on `0:00` for the entire preview.
  return `${minutes}:${rest.toFixed(1).padStart(4, '0')}`;
}
