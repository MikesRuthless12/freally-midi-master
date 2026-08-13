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
import type { Lane, SplitPart } from '../lib/ipc-types';

/** One row in the browser. Mirrors `explorer::Entry`. */
export type ExplorerEntry = {
  name: string;
  path: string;
  isDir: boolean;
  /**
   * ⛔⛔ **Two kinds, two sets of affordances** — TASK-058's rule, carried as a
   * field so no call site has to re-derive it from the extension. An audio file
   * gets the waveform, the audition and a drum pad. A `.mid` gets playback and
   * drag-to-generator; **a waveform for a `.mid`, or a pad assignment for one, is
   * a control that can only fail.**
   */
  kind: 'dir' | 'audio' | 'midi';
};

/**
 * How many library folders may be open at once.
 *
 * ⛔ Mike's number, 2026-08-10: *"i should be able to have up to 8 folders in the
 * view … and if you want to add more, then you have to exit out of one of them."*
 * ⚠ **Mirrors `explorer::MAX_ROOTS` and does not replace it.** This one disables
 * the button so the rule is visible before it is hit; the plugin refuses a ninth
 * however it arrives.
 */
export const MAX_ROOTS = 8;

/**
 * One starred file (TASK-058C). Mirrors `favourites::Favourite`.
 *
 * ⛔ **Keyed by path, and the roadmap says hash.** `plugin/src/favourites.rs`
 * carries the reasoning: drawing a star on every row cannot mean hashing every
 * file in the folder, and Mike's own description — *"it will delete the memory of
 * what folder they are in"* — is about the path. The cost is real: a starred file
 * that moves loses its star.
 */
export type Favourite = {
  path: string;
  name: string;
  kind: 'audio' | 'midi';
};

/**
 * One file the browser opened lately (TASK-058). Mirrors `recent::Recent`.
 *
 * ⚠ **The same shape as [`Favourite`] deliberately**, so one row component draws
 * both lists. They are separate *types* because they answer different questions —
 * a star is a choice and a history is a record — and collapsing them into one
 * would make "clear the history" reach for the favourites too.
 */
export type Recent = {
  path: string;
  name: string;
  kind: 'audio' | 'midi';
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

/**
 * `\\?\C:\Samples` and `C:\Samples` are the same folder.
 *
 * ⛔⛔ **The two spellings have caused three bugs on this path now**, and
 * `same_or_inside`'s own doc records the first two. Roots are stored exactly as
 * the producer added them; everything the plugin *derives* goes through
 * `canonicalize`, which on Windows answers with the `\\?\` prefix. So a root and
 * the folder inside it that the plugin reports as current are routinely written
 * two different ways, and every `===` or `startsWith` between them is wrong.
 *
 * ⚠ The third was the one Mike hit on 2026-08-10 — see `refuse_remote`, where
 * the same prefix was being read as a *network* path and stopped the browser
 * descending at all.
 */
function normalizePath(path: string): string {
  return path.replace(/^\\\\\?\\/, '').replace(/[\\/]+$/, '');
}

/** Is `path` the same folder as `other`, whichever way each is spelled? */
export function samePath(path: string | null, other: string | null): boolean {
  if (path === null || other === null) return false;
  // ⚠ Case-insensitively, because Windows paths are — and the plugin may answer
  // with a drive letter cased differently from the one the producer typed.
  return normalizePath(path).toLowerCase() === normalizePath(other).toLowerCase();
}

/**
 * Is `path` somewhere below `folder`?
 *
 * ⚠ **A separator is required after the prefix**, or `…/Kicks` would count
 * `…/Kicks-old` as one of its own children — the classic prefix-matching bug,
 * and the reason the Rust side compares component by component rather than by
 * string. This is only ever used to tidy the *view*, never to decide access;
 * containment as a security boundary stays in `Explorer::contains`.
 */
export function isInside(path: string, folder: string): boolean {
  const inner = normalizePath(path).toLowerCase();
  const outer = normalizePath(folder).toLowerCase();
  return inner.startsWith(`${outer}/`) || inner.startsWith(`${outer}\\`);
}

/**
 * The folder names between `folder` and `path`, excluding the file itself.
 *
 * ⛔⛔ **Because slicing by `root.path.length` was wrong, and the fixture hid
 * it.** `goToFavourite` walked down to a starred file with
 * `path.slice(root.path.length)` — but a root is stored **as the producer added
 * it** (`C:\lib\Samples`) while a favourite's path came from a row the plugin had
 * **canonicalised** (`\\?\C:\lib\Samples\Kicks\kick.wav`). The offset is wrong by
 * the length of the `\\?\` prefix, so the walk expanded garbage segments and
 * "take me to the exact spot" landed nowhere. The e2e fixture uses `/library/…`,
 * where raw and canonical coincide, so nothing went red.
 *
 * ⚠ **Every path decomposition goes through here**, for the reason `samePath`
 * exists: the two spellings are the bug this module keeps paying for, and a
 * `slice` or a `+` on a path at a call site is how the next one arrives.
 */
export function segmentsUnder(folder: string, path: string): string[] {
  if (!isInside(path, folder)) return [];
  // ⚠ Measured on the **normalised** forms, so the offset matches the string it
  // is being taken from.
  const inner = normalizePath(path);
  const outer = normalizePath(folder);
  return inner.slice(outer.length).split(/[\\/]/).filter(Boolean);
}

/*
 * ⚠ **There is deliberately no `joinPath` here.** One existed for a day and it
 * was the bug: building a child path by joining a name onto a parent produces the
 * *parent's* spelling, and the plugin answers in its own (canonicalised) one — so
 * the two never matched. Every path this page holds comes from a row the plugin
 * returned. If you need a child's path, look it up in `children`; if it is not
 * there, the folder has not been read yet.
 */

/**
 * What kind of file a path names, from its extension alone.
 *
 * ⚠ **Mirrors `engine::formats::kind_of`**, deliberately and knowingly: the page
 * has to answer this before any command replies — see `select` — and a round trip
 * to ask the plugin would be exactly the async gap that made the old code report
 * a `.mid` as audio. The lists are short and the *authority* stays in Rust; this
 * decides which question to ask, never what the file is.
 */
export function kindOfPath(path: string): 'audio' | 'midi' {
  return /\.midi?$/i.test(path) ? 'midi' : 'audio';
}

/**
 * The deepest folder currently expanded, which is the one `Up` and `←` shut.
 *
 * ⚠ **Deepest, not last-clicked.** Opening a sibling after opening a child
 * leaves the child open and more deeply nested, so "the one I opened most
 * recently" would retract the wrong branch.
 */
export function innermostExpanded(expanded: string[]): string | null {
  if (expanded.length === 0) return null;
  return expanded.reduce((deepest, held) => (isInside(held, deepest) ? held : deepest));
}

/**
 * The folder the producer is standing in — the one a re-roll draws from.
 *
 * ⛔⛔ **Mike, 2026-08-11:** *"if i am on an actual sample in the file explorer,
 * or i am on a folder in file explorer, either way, it should remember the
 * folder's name that i am in, and it should randomize a sample from that
 * specific folder for the 'Re-roll'."*
 *
 * ▶ **Two selections, one answer.** A *file* means the folder it is in; a
 * *folder* means itself. Before this the plugin answered from its own "current
 * folder", which the tree view stopped maintaining the moment navigation became
 * expand-in-place — so the dice re-rolled from wherever the old list had last
 * been opened, or from nothing.
 *
 * ⚠ **Falls back to the deepest expanded branch**, so the dice still has a
 * sensible source before anything has been clicked. `null` only when the browser
 * is closed or empty, and the plugin then keeps its own answer.
 *
 * ⚠ **Derived here rather than in Rust**, because the *selection* lives here.
 * Deriving it on the other side would be a second idea of where the producer is
 * standing, and the two would part company the first time one of them changed.
 */
export function standingIn(state: {
  selected: string | null;
  selectedKind: 'audio' | 'midi' | null;
  expanded: string[];
}): string | null {
  if (state.selected !== null) {
    // A selection is always a file — folders are expanded, not selected — so
    // this is its parent. Windows and POSIX separators both, because a root the
    // producer typed and a path the plugin canonicalised are spelled differently
    // (see `normalizePath`).
    const at = Math.max(state.selected.lastIndexOf('/'), state.selected.lastIndexOf('\\'));
    if (at > 0) return state.selected.slice(0, at);
  }
  return innermostExpanded(state.expanded);
}

type ExplorerStore = {
  roots: ExplorerEntry[];
  /**
   * The folder a per-pad randomise draws from.
   *
   * ⚠ **`parent`, `entries` and `truncated` are deliberately NOT here.** The
   * plugin still sends them — `ExplorerReply` mirrors `explorer::State` — but the
   * tree draws `children`/`truncatedIn` and `Up` collapses instead of navigating,
   * so nothing read them. Keeping them would be two representations of "the rows
   * in a folder" with only one live, which is the first thing a reader reaches
   * for when adding a feature.
   */
  folder: string | null;
  /**
   * The rows inside each folder the producer has expanded, keyed by the path
   * that was *asked for*.
   *
   * ⚠ **Keyed by the request, not by the reply.** `explorer_list` canonicalises
   * on the way in, so a root asked for as `C:\Samples` answers with rows under
   * `\\?\C:\Samples`. Keying by what came back would mean the next lookup — made
   * with the root's own spelling — missed, and the node would re-fetch on every
   * render.
   */
  children: Record<string, ExplorerEntry[]>;
  /** Which folders are open, as a stable array — see the note in `PadGrid`. */
  expanded: string[];
  /**
   * Which library folder's tab is showing, or `null` for the first one.
   *
   * ⚠ **Not persisted, and that matches the roots' own rule**: the library is a
   * setup, the tab you were last on is a position. `Explorer::state`'s comment
   * makes the same distinction about the browse location.
   */
  activeRoot: string | null;
  /** Folders whose listing hit the plugin's row cap. */
  truncatedIn: string[];
  /** Starred samples, one-shots and MIDI. Mirrors `favourites::Favourite`. */
  favourites: Favourite[];
  /**
   * The files the browser opened lately, newest first (TASK-058).
   *
   * ⛔ **Read-only from here.** The plugin records an entry when it auditions a
   * sample or reads a `.mid`; nothing on the page adds to it. See the
   * `recent_list` handler for why there is no `recent_add`.
   */
  recent: Recent[];
  /**
   * What the selected `.mid` was found to contain, or `null`.
   *
   * ⛔ **Read when the file is selected, not when a button is pressed.** The
   * whole point is that a producer sees *what is in the file* before deciding
   * where it goes — a "Split" button that only reveals its answer after you
   * commit is the shape TASK-058D refuses: it would present a guess as a result.
   */
  midiSplit: SplitPart[] | null;
  /** Whether a folder dialog is open — drives the poll in `addFolder`. */
  picking: boolean;
  /** Whether `explorer_state` has ever answered — "empty" is not "not asked". */
  loaded: boolean;
  /** The file being auditioned, or `null`. */
  selected: string | null;
  /**
   * What kind the selected file is — set synchronously with `selected`.
   *
   * ⛔ **Not inferred from `midiSplit`.** Two consumers used to sniff
   * `midiSplit !== null`, which is `null` for the whole window between the click
   * and the reply *and permanently when the split fails* — so a `.mid` reported
   * as audio, the pad offered to take it and the transport drew for it. See
   * `select`.
   */
  selectedKind: 'audio' | 'midi' | null;
  /**
   * The starred paths, as a set.
   *
   * ⚠ **Beside `favourites` rather than derived per row.** `FileTree` asked
   * `favourites.some(…)` on every row, which is O(rows × favourites) — 2,000
   * rows against 500 stars is a million string comparisons **per render**, and
   * the tree re-renders on every refresh. Built once where the list is written.
   */
  starred: Set<string>;
  waveform: Waveform | null;
  position: PreviewPosition;
  error: string | null;
  /** The left rail's width in CSS pixels, persisted. */
  railWidth: number;

  refresh: () => Promise<void>;
  /** Read the starred list. ⚠ At mount and after a star — never on the poll. */
  loadFavourites: () => Promise<void>;
  /** Re-read the history. Cheap, and only worth doing when it can have moved. */
  loadRecent: () => Promise<void>;
  /** Forget where you have been. */
  clearRecent: () => Promise<void>;
  addFolder: () => Promise<void>;
  removeFolder: (path: string) => Promise<void>;
  /** Open a folder's twisty if it is shut, shut it if it is open. */
  toggleFolder: (path: string) => Promise<void>;
  /** Shut a folder's twisty. What `Up` and `←` do to the folder you are in. */
  collapse: (path: string) => void;
  /** Show a different library folder's tab. */
  setActiveRoot: (path: string) => void;
  /** Star a file, or unstar it if it already is. */
  toggleFavourite: (path: string) => Promise<void>;
  /**
   * Go to a starred file: reveal it in the tree if its folder is still open in
   * the browser, and in the OS file manager if it is not.
   */
  goToFavourite: (path: string) => Promise<void>;
  select: (path: string) => Promise<void>;
  play: () => Promise<void>;
  pause: () => Promise<void>;
  stop: () => Promise<void>;
  seek: (seconds: number) => Promise<void>;
  toggleLoop: () => Promise<void>;
  setReverse: (on: boolean) => Promise<void>;
  /** Put the selected sample on a drum lane. */
  /**
   * Put the selected sample on a drum lane.
   *
   * ⛔ **`reversed` plays the one-shot backwards** — Mike, 2026-08-11: *"Ctrl +
   * left arrow when you have a sample selected should add the sample to that
   * selected drum pad lane in reverse and Ctrl + right arrow should add the
   * sample to that selected drum pad lane playing regularly."* It travels to the
   * plugin and is stored with the project, so a reopened session does not
   * silently play it forwards.
   */
  dropOn: (lane: Lane, path: string, reversed?: boolean) => Promise<void>;
  setRailWidth: (px: number) => void;
};

export const useExplorer = create<ExplorerStore>((set, get) => ({
  roots: [],
  folder: null,
  children: {},
  expanded: [],
  truncatedIn: [],
  activeRoot: null,
  favourites: [],
  recent: [],
  starred: new Set(),
  midiSplit: null,
  picking: false,
  loaded: false,
  selected: null,
  selectedKind: null,
  waveform: null,
  position: STOPPED,
  error: null,
  railWidth: loadRailWidth(),

  async refresh() {
    try {
      const reply = await invoke<ExplorerReply>('explorer_state');
      // ⚠ **Only the library, the current folder and the dialog flag.** The reply
      // still carries `parent`/`entries`/`truncated` because it mirrors
      // `explorer::State`, and nothing reads them — see the note on `folder`.
      set({
        roots: reply.roots,
        folder: reply.folder,
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

  /**
   * Read the starred list from disk.
   *
   * ⛔ **Not inside `refresh`, and that is a real cost rather than tidiness.**
   * `refresh` runs on the folder-dialog poll — every 400 ms for as long as a
   * producer browses for a folder — and on every twisty click. Reading and
   * parsing `favourites.json` on each of those is ~2.5 file reads a second on the
   * host's editor thread, for a list that changes only when somebody presses a
   * star. ⚠ And `toggleFavourite` is already handed the fresh list back by the
   * plugin, so the only time this is needed is at mount.
   */
  async loadFavourites() {
    try {
      const favourites = await invoke<Favourite[]>('favourites_list');
      set({ favourites, starred: new Set(favourites.map((held) => held.path)) });
    } catch {
      // No stars is not a broken browser — the listing is unaffected.
    }
  },

  // ⚠ **Not on `refresh`**, for the reason `loadFavourites` gives above: that
  // runs every 400 ms while the folder dialog is open, and the history changes
  // only when something is auditioned. It is read at mount and after a select
  // that actually loaded something.
  async loadRecent() {
    try {
      set({ recent: await invoke<Recent[]>('recent_list') });
    } catch {
      // No history is not a broken browser.
    }
  },

  async clearRecent() {
    try {
      set({ recent: await invoke<Recent[]>('recent_clear') });
    } catch (error) {
      set({ error: reason(error) });
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

    // ⚠ The removed root's branch goes with it. Left behind, its rows would
    // still be in `children` and would reappear the moment a folder of the same
    // name was added back — showing a listing nothing had re-read.
    const children = { ...get().children };
    for (const key of Object.keys(children)) {
      if (samePath(key, path) || isInside(key, path)) delete children[key];
    }
    set({
      children,
      expanded: get().expanded.filter((held) => !samePath(held, path) && !isInside(held, path)),
    });
  },

  /**
   * Expand or collapse one node of the tree.
   *
   * ⛔⛔ **A tree, because a one-folder-at-a-time list is not a file explorer.**
   * Mike, 2026-08-10: *"you need to show it like a real file explorer does, where
   * the main folder is at the top, and then indents and shows the subfolders
   * underneath and then the files beneath that."*
   *
   * ⛔ **Expanding also makes the folder current, and collapsing does not undo
   * that.** `explorer_open` is what per-pad randomise draws its samples from, so
   * clicking a folder has to mean "I am working here" as well as "show me". But
   * shutting a twisty is a change to what is *drawn*, and silently re-pointing
   * the dice at the parent because a producer tidied the view would be a
   * side effect nobody asked for.
   *
   * ⚠ **Rows are re-read every time it is opened**, not cached across a close.
   * A producer who drops a sample into a folder from Explorer and comes back
   * expects to see it; a cache that outlived the twisty would show them the
   * folder as it was when they first looked.
   */
  async toggleFolder(path) {
    const open = get().expanded.some((held) => samePath(held, path));
    if (open) {
      get().collapse(path);
      return;
    }

    set({ error: null, expanded: [...get().expanded, path] });
    // ⚠ Made current as well — see above. Failure here is not fatal to the
    // expansion: the rows are what the producer asked to see.
    void invoke('explorer_open', { path }).then(
      () => get().refresh(),
      () => {},
    );

    try {
      const reply = await invoke<ExplorerReply>('explorer_list', { path });
      set({
        children: { ...get().children, [path]: reply.entries },
        truncatedIn: reply.truncated
          ? [...get().truncatedIn.filter((held) => !samePath(held, path)), path]
          : get().truncatedIn.filter((held) => !samePath(held, path)),
      });
    } catch (error) {
      // ⛔ Shut again on failure, rather than left open and empty. An expanded
      // twisty over no rows reads as "this folder is empty", which is a claim
      // the page has not earned — the read is what failed.
      set({
        error: reason(error),
        expanded: get().expanded.filter((held) => !samePath(held, path)),
      });
    }
  },

  async toggleFavourite(path) {
    const starred = get().starred.has(path);
    set({ error: null });
    try {
      const favourites = await invoke<Favourite[]>(
        starred ? 'favourites_remove' : 'favourites_add',
        { path },
      );
      // ⚠ The plugin answers with the whole list rather than an acknowledgement,
      // so the page never has to guess what its own write produced — the same
      // shape `kits_save` uses. The set is rebuilt here for the same reason it
      // exists: the tree asks it per row.
      set({ favourites, starred: new Set(favourites.map((held) => held.path)) });
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  /**
   * ⛔⛔ **Two destinations, and which one is not a preference.** Mike,
   * 2026-08-10: *"it will take us to the exact spot of that exact sample in the
   * actual 'File Explorer' browser and if it's not a folder that you still have
   * in the 'File Explorer' then it should take you there in Windows Explorer or
   * the macOS Explorer, so all 'Starred Favorites' should be able to be reachable
   * at any given time."*
   *
   * So: inside the library, reveal it in the tree. Outside it, hand it to the OS.
   * The second is what makes "reachable at any given time" true after a producer
   * closes the folder it lives in — which they will, with only eight tabs.
   */
  async goToFavourite(path) {
    set({ error: null });
    const root = get().roots.find((entry) => isInside(path, entry.path));

    if (root === undefined) {
      // Not in any open library folder — the OS knows where it is.
      try {
        await invoke('favourites_reveal', { path });
      } catch (error) {
        set({ error: reason(error) });
      }
      return;
    }

    // ⛔ **Every folder between the root and the file, in order.** Expanding only
    // the immediate parent would leave it inside a branch that is still shut, so
    // "take me to the exact spot" would land on nothing.
    //
    // ⛔⛔ **The walk follows the ROWS the plugin returned, never a path this
    // rebuilds.** It used to join the next name onto the root's own spelling —
    // and a root is stored as the producer added it (`C:\lib\Samples`) while
    // every row under it comes back **canonicalised** (`\\?\C:\lib\Samples\…`).
    // So `toggleFolder` filed the rows under `C:\lib\Samples\Kicks` while
    // `FileTree` looked them up as `\\?\C:\lib\Samples\Kicks`: the branch opened,
    // read "Reading…" underneath forever, and the starred file was never
    // revealed — the one thing the feature exists to do. Found by a code review,
    // 2026-08-10; the e2e fixture uses `/library/…`, where the two spellings
    // coincide, so nothing went red.
    //
    // ⚠ Names come from `segmentsUnder`; the *paths* come from `children`.
    set({ activeRoot: root.path });
    const between = segmentsUnder(root.path, path);
    const isOpen = (folder: string) => get().expanded.some((held) => samePath(held, folder));

    let at = root.path;
    if (!isOpen(at)) await get().toggleFolder(at);

    // ⚠ `slice(0, -1)` drops the filename: a file has no twisty to open.
    for (const name of between.slice(0, -1)) {
      const next = (get().children[at] ?? []).find((row) => row.isDir && row.name === name);
      // ⚠ The folder is gone from disk since it was starred. Stop where we are
      // rather than walking a path the plugin will refuse — the producer is left
      // looking at the deepest folder that does still exist.
      if (next === undefined) return;
      at = next.path;
      // ⚠ Awaited in sequence: each level's rows have to be read before the next
      // level's row can be found in them.
      if (!isOpen(at)) await get().toggleFolder(at);
    }

    await get().select(path);
  },

  setActiveRoot(path) {
    // ⚠ **The other tab's branches are left expanded.** Coming back to a folder
    // you were three levels into and finding it shut is the browser forgetting
    // what you were doing; the rows are already in `children` and cost nothing
    // to keep.
    set({ activeRoot: path, error: null });
  },

  collapse(path) {
    // ⛔ **Descendants too.** Leaving them in `expanded` means re-opening a
    // folder silently re-opens every branch that was under it — the producer
    // collapsed a tree and it grew back.
    set({
      expanded: get().expanded.filter((held) => !samePath(held, path) && !isInside(held, path)),
    });
  },

  async select(path) {
    // ⛔⛔ **The kind is DERIVED, not passed in, and it is stored.**
    //
    // It used to be a parameter defaulting to `'audio'`, and two things went
    // wrong at once. The panel's arrow-key handler called `select(path)` with no
    // kind, so walking onto a `.mid` with → asked for a *waveform of a MIDI
    // file* — the one thing the two-kinds rule exists to prevent. And nothing
    // stored what the selected file was: `PadGrid` and `PreviewPlayer` both
    // sniffed `midiSplit !== null` instead, which is `null` for the whole window
    // between the click and the reply — and **permanently** when the split fails.
    // In that state a `.mid` reported as audio, so the pad offered to take it and
    // the transport drew for it.
    //
    // ⚠ The extension is the only input either question ever had; deriving it
    // here means one answer, set synchronously, before any await.
    const kind = kindOfPath(path);
    // ⛔ **Set before either call, so the panel switches immediately** and a
    // slow decode cannot leave the previous sample's waveform on screen looking
    // like the one that was just clicked.
    set({ selected: path, selectedKind: kind, waveform: null, error: null });

    // ⛔⛔ **A `.mid` has no waveform and no PCM, so neither is asked for.**
    // `explorer_waveform` refuses anything that is not audio and `preview_load`
    // would too — asking anyway would put a refusal on screen every time a
    // producer clicked a MIDI file, which is the panel telling them something is
    // broken about a file that is perfectly fine. TASK-058's two-kinds rule, at
    // the one place that would otherwise treat them alike.
    //
    // ⚠ Selecting it still means something: it is what the arrow keys and
    // drag-to-generator act on. Hearing it is TASK-058's MIDI audition, which is
    // not built yet — so today this selects and draws nothing rather than
    // claiming to play.
    if (kind === 'midi') {
      set({ midiSplit: null });
      try {
        const parts = await invoke<SplitPart[]>('explorer_midi_split', { path });
        // A late reply must not describe a file that is no longer selected —
        // the same rule the waveform follows two branches down.
        if (get().selected !== path) return;
        set({ midiSplit: parts });
        // The plugin wrote the entry; this is the page catching up. Only here
        // and after an audio load — the two places a file is really opened.
        void get().loadRecent();
      } catch (error) {
        if (get().selected !== path) return;
        set({ error: reason(error) });
      }
      return;
    }

    // ⚠ Cleared on the way into an audio file, or the panel would go on offering
    // to split the `.mid` looked at before it.
    if (get().midiSplit !== null) set({ midiSplit: null });

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
      void get().loadRecent();
    } catch (error) {
      if (get().selected !== path) return;
      set({ error: reason(error), waveform: null });
      // ⛔⛔ **AND NOTHING PLAYS, BECAUSE NOTHING WAS LOADED.** `preview_load` is
      // in the `Promise.all` above, so this catch is also the case where the
      // plugin still holds the *previous* file. Falling through to
      // `preview_play` started that one — so landing on a corrupt `.wav` showed
      // an error and a blank waveform for the file under the cursor while the
      // last good sample sounded, and the transport said `playing` for a file
      // that was not the one being heard.
      return;
    }

    // ⛔⛔ **LANDING ON A FILE PLAYS IT** (Mike, 2026-08-11): *"the files need to
    // play as you go up and down in the list with the up/down arrow or by
    // clicking on them."*
    //
    // ⚠ **This reverses a documented judgement, and the note it reverses is
    // still in `ExplorerPanel`'s arrow handler.** That one argued auditioning on
    // every step *"would make ↓ unusable for simply getting past a folder"* and
    // paired ↓ with → instead — step onto a row, press → to hear it. Mike has
    // now asked for the other behaviour by name, twice over (arrows *and*
    // clicks), so the two-key gesture is gone and → keeps working for anyone who
    // still reaches for it.
    //
    // ⛔ **In `select`, not in the two callers.** A click goes through
    // `FileTree`, an arrow through `ExplorerPanel`, and a starred file through
    // `reveal` — three doors onto one behaviour, and putting it behind them all
    // is how one of them ends up silent.
    //
    // ⚠ **Guarded on the selection, because a fast walk overlaps.** Holding ↓
    // starts a load per row and they resolve out of order; without this the
    // *previous* row's reply would start the *current* row's sample a second
    // time. `preview_load` replaces what is loaded, so the last one to land is
    // the one that sounds.
    if (get().selected !== path) return;
    // ⛔ **Silent on failure, unlike the transport buttons.** An audition is
    // feedback on a gesture that has already happened — the same argument
    // `DrumGrid/audition.ts` spells out. Putting `play`'s error on screen here
    // would mean a browser with no audio thread shows a failure every time a
    // producer clicks a file, for a click that did nothing wrong.
    try {
      await invoke('preview_play');
      if (get().selected !== path) return;
      poke();
      set({ position: { ...get().position, playing: true } });
    } catch {
      // See above.
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

  async dropOn(lane, path, reversed = false) {
    set({ error: null });
    try {
      await invoke('explorer_drop', { lane, path, reversed });
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
