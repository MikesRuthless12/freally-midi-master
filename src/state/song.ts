/**
 * Song Mode's document (TASK-063A / TASK-063B).
 *
 * Separate from `session.ts` on purpose: a session holds *one pattern* and the
 * inputs that made it, and a song holds an arrangement of many. Folding the
 * song into that slot would mean generating a song replaced whatever the roll
 * was showing, and switching tabs would look like the editor had lost the work.
 *
 * ⛔ **Every edit goes through the pure functions in
 * `components/SongTimeline/clips.ts`.** The tiling invariant — sections end to
 * end, no gap, no overlap — is enforced in exactly one place, and a store method
 * that spliced `sections` itself would be the second place it could break.
 */

import { create } from 'zustand';

import { invoke } from '../lib/ipc';
import type { Part, Song } from '../lib/ipc-types';
import {
  cloneSection,
  copyClips,
  deleteClips,
  isSelected,
  pasteClips,
  resizeSection,
  sameClip,
  type Clipboard,
  type ClipId,
} from '../components/SongTimeline/clips';
import { zoomIn, zoomOut, type View } from '../components/SongTimeline/geometry';
import {
  noteDocumentChange,
  reason,
  registerSongDocument,
  useSession,
  type SessionPins,
} from './session';

export type SongState = {
  song: Song | null;
  generating: boolean;
  error: string | null;
  /** True once the arrangement has been edited away from what was generated. */
  edited: boolean;

  view: View;
  selection: ClipId[];
  clipboard: Clipboard | null;
  /**
   * The section the last selection was made in.
   *
   * ⛔ Kept separately from `selection` because `cut()` empties the selection.
   * Deriving the paste target from `selection[0]` meant Ctrl+X then Ctrl+V read
   * an empty list, fell back to 0, and dropped the cut clips onto the *first*
   * section instead of putting them back where they came from.
   */
  anchor: number | null;

  /**
   * Clips the producer has pinned, as `sectionIndex:part` (TASK-070).
   *
   * ⛔ **A flat set of cells, not a tree of section / row / cell locks.** All
   * three gestures the roadmap names resolve to the same question a re-roll
   * asks — *may this clip change?* — so locking a row is locking its cells and
   * locking a section is locking its column. Keeping three kinds of lock would
   * mean answering that question three ways, and `reroll` would have to consult
   * all of them in an order nobody wrote down.
   */
  locks: string[];

  generate: (args: {
    styleId: string;
    seed: string;
    pins: SessionPins;
    mood: string | null;
  }) => Promise<void>;

  /** Re-roll one section, keeping every locked clip (TASK-067 / TASK-071). */
  reroll: (index: number, mood: string | null) => Promise<void>;

  toggleLock: (clip: ClipId) => void;
  toggleSectionLock: (index: number) => void;
  toggleRowLock: (part: Part) => void;

  zoomIn: () => void;
  zoomOut: () => void;

  select: (clip: ClipId, additive: boolean) => void;
  selectSection: (index: number, additive: boolean) => void;
  clearSelection: () => void;

  resize: (index: number, bars: number) => void;
  clone: (index: number) => void;
  deleteSelection: () => void;
  copy: () => void;
  cut: () => void;
  paste: () => void;
};

const INITIAL_VIEW: View = { zoom: 24, scrollBar: 0 };

export const useSong = create<SongState>((set, get) => ({
  song: null,
  generating: false,
  error: null,
  edited: false,
  view: INITIAL_VIEW,
  selection: [],
  clipboard: null,
  anchor: null,
  locks: [],

  async generate({ styleId, seed, pins, mood }) {
    if (get().generating) return;
    set({ generating: true, error: null });
    try {
      const song = await invoke<Song>('generate_song', {
        request: {
          styleId,
          // An empty box means "pick one for me"; "" would be a seed that fails
          // to parse rather than an absent one.
          seed: seed === '' ? null : seed,
          session: pins,
          mood,
        },
      });
      // A fresh generation *is* the seed's own output again, so the document
      // goes back to being describable by its inputs — the same rule
      // `session.generate` follows for a pattern.
      set({
        song,
        generating: false,
        edited: false,
        selection: [],
        anchor: null,
        // ⛔ Locks go with the arrangement they were placed on. A lock names a
        // section index and a part, and a fresh generation has neither the same
        // sections nor the same clips — so a kept lock would pin whatever
        // happened to land at that index, which is not what the producer pinned.
        locks: [],
        // The view is deliberately kept: regenerating while zoomed in should
        // not throw the producer back to the top of the song.
      });
      // A fresh song is describable by its seed again, so nothing has to be
      // stored — but the *previous* one may have been edited and saved, and
      // leaving that in the project file would reopen it over this one.
      noteDocumentChange();
    } catch (error) {
      set({ generating: false, error: reason(error) });
    }
  },

  async reroll(index, mood) {
    const { song, generating, locks } = get();
    if (!song || generating) return;
    set({ generating: true, error: null });
    try {
      const next = await invoke<Song>('reroll_section', {
        request: {
          song,
          index,
          // Absent is "pick one for me", the same rule the seed box follows.
          // A re-roll always wants a new one — that is the gesture.
          seed: null,
          locked: lockedPartsIn(locks, index),
          mood,
        },
      });
      set({ song: next, generating: false, selection: [], anchor: null });
      // ⛔ A re-rolled section is no longer what the song's own seed produces,
      // so from here the arrangement only exists if it is saved. This is the
      // edit that is easiest to lose, because nothing about it *looks* like an
      // edit — the timeline redraws and the geometry is unchanged.
      markEdited();
    } catch (error) {
      set({ generating: false, error: reason(error) });
    }
  },

  toggleLock(clip) {
    const key = lockKey(clip);
    const { locks } = get();
    set({ locks: locks.includes(key) ? locks.filter((l) => l !== key) : [...locks, key] });
  },

  toggleSectionLock(index) {
    const { song, locks } = get();
    const section = song?.sections[index];
    if (!section) return;
    const keys = (Object.keys(section.patterns) as Part[]).map((part) =>
      lockKey({ sectionIndex: index, part }),
    );
    // Locked only when *every* clip in it is: a half-locked section that
    // reported itself locked would let a re-roll change part of it.
    const locked = keys.every((key) => locks.includes(key));
    set({
      locks: locked
        ? locks.filter((l) => !keys.includes(l))
        : [...locks.filter((l) => !keys.includes(l)), ...keys],
    });
  },

  toggleRowLock(part) {
    const { song, locks } = get();
    if (!song) return;
    const keys = song.sections
      .map((section, index) =>
        section.patterns[part] ? lockKey({ sectionIndex: index, part }) : null,
      )
      .filter((key): key is string => key !== null);
    if (keys.length === 0) return;
    const locked = keys.every((key) => locks.includes(key));
    set({
      locks: locked
        ? locks.filter((l) => !keys.includes(l))
        : [...locks.filter((l) => !keys.includes(l)), ...keys],
    });
  },

  zoomIn() {
    set({ view: { ...get().view, zoom: zoomIn(get().view.zoom) } });
  },
  zoomOut() {
    set({ view: { ...get().view, zoom: zoomOut(get().view.zoom) } });
  },

  select(clip, additive) {
    const { selection } = get();
    if (!additive) {
      set({ selection: [clip], anchor: clip.sectionIndex });
      return;
    }
    // Additive click toggles, so a mis-shift-click is undone by repeating it
    // rather than by starting the selection over.
    set({
      selection: isSelected(selection, clip)
        ? selection.filter((s) => !sameClip(s, clip))
        : [...selection, clip],
      anchor: clip.sectionIndex,
    });
  },

  selectSection(index, additive) {
    const song = get().song;
    if (!song) return;
    const section = song.sections[index];
    if (!section) return;
    const clips = (Object.keys(section.patterns) as Part[]).map((part) => ({
      sectionIndex: index,
      part,
    }));
    set({ selection: additive ? [...get().selection, ...clips] : clips, anchor: index });
  },

  clearSelection() {
    set({ selection: [] });
  },

  resize(index, bars) {
    apply(set, get, (song) => resizeSection(song, index, bars));
  },
  clone(index) {
    // ⛔ The selection is dropped rather than carried: every section after the
    // insert has shifted by one, so a selection held by index now names
    // different clips. Keeping it would silently move the *next* delete onto
    // something the producer never selected.
    apply(set, get, (song) => cloneSection(song, index), true);
  },

  deleteSelection() {
    const { selection } = get();
    if (selection.length === 0) return;
    apply(set, get, (song) => deleteClips(song, selection), true);
  },

  copy() {
    const { song, selection } = get();
    if (!song) return;
    const clipboard = copyClips(song, selection);
    if (clipboard) set({ clipboard });
  },

  cut() {
    get().copy();
    // Only cuts what actually reached the clipboard, so a failed copy cannot
    // delete the thing it failed to copy.
    if (get().clipboard) get().deleteSelection();
  },

  paste() {
    const { clipboard, anchor } = get();
    if (!clipboard) return;
    apply(set, get, (song) => pasteClips(song, clipboard, anchor ?? 0));
  },
}));

/**
 * Drop the arrangement when the producer picks a different artist.
 *
 * ⛔ **`session.select` already does this for the pattern, and its comment says
 * why: an artist's work left on screen under another artist's name is the most
 * convincing wrong thing the app can show.** A song is the same claim at a
 * larger scale — the whole arrangement, its tempo, its key and its pattern store
 * all belong to the artist it was built for, and every edit and every export
 * afterwards would operate on that one. `session.select` cannot clear it
 * directly (this store imports *from* session, so the dependency only runs one
 * way), which is why it is a subscription rather than a line in `select`.
 */
useSession.subscribe((state, previous) => {
  if (state.selectedId !== previous.selectedId) {
    useSong.setState({
      song: null,
      selection: [],
      anchor: null,
      clipboard: null,
      edited: false,
      locks: [],
      error: null,
    });
  }
});

/**
 * Publish the arrangement to the session, which is what the host saves.
 *
 * ⛔ **Registered rather than imported, because the dependency runs one way.**
 * See `registerSongDocument` in `session.ts`: this module already imports
 * `useSession`, so `session.ts` reaching back for `useSong` would be a cycle
 * between two stores whose initialisation order the bundler chooses.
 */
registerSongDocument(
  () => {
    const { song, edited } = useSong.getState();
    return { song, edited };
  },
  ({ song, edited }) =>
    useSong.setState({
      song,
      edited,
      // A restored arrangement is not one anybody is mid-gesture on.
      selection: [],
      anchor: null,
      clipboard: null,
      locks: [],
      error: null,
    }),
);

/** The key one clip is locked under. */
function lockKey({ sectionIndex, part }: ClipId): string {
  return `${sectionIndex}:${part}`;
}

/**
 * The parts locked in one section, in the shape the engine's re-roll wants.
 *
 * The engine is deliberately lock-agnostic: it takes a list of parts to leave
 * alone, and everything about *how* a producer expressed that — a cell, a row,
 * a whole section — is resolved here.
 */
function lockedPartsIn(locks: string[], index: number): Part[] {
  const prefix = `${index}:`;
  return locks
    .filter((lock) => lock.startsWith(prefix))
    .map((lock) => lock.slice(prefix.length) as Part);
}

/**
 * Record that the arrangement has moved away from its seed, and save it.
 *
 * ⛔ **The two happen together and always have to.** `edited` is what makes the
 * song worth storing at all — `send()` skips an unedited one deliberately — so
 * a path that set the flag without asking for a save would leave the producer's
 * arrangement in memory only, which is the failure this task exists to close.
 */
function markEdited(): void {
  useSong.setState({ edited: true });
  noteDocumentChange();
}

/**
 * Run an edit and record that the arrangement has moved away from its seed.
 *
 * The pure functions return the *same object* when they change nothing, which
 * is what makes `edited` honest: a resize to the width it already had is not an
 * edit, and marking it as one would tell the producer their song no longer
 * matches its seed when it does.
 */
function apply(
  set: (partial: Partial<SongState>) => void,
  get: () => SongState,
  edit: (song: Song) => Song,
  clearSelection = false,
) {
  const { song } = get();
  if (!song) return;
  const next = edit(song);
  if (next === song) return;
  set({
    song: next,
    edited: true,
    ...(clearSelection ? { selection: [] } : {}),
  });
  noteDocumentChange();
}
