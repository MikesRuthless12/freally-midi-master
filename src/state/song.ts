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
import { reason, type SessionPins } from './session';

export type SongState = {
  song: Song | null;
  generating: boolean;
  error: string | null;
  /** True once the arrangement has been edited away from what was generated. */
  edited: boolean;

  view: View;
  selection: ClipId[];
  clipboard: Clipboard | null;

  generate: (args: {
    styleId: string;
    seed: string;
    pins: SessionPins;
    mood: string | null;
  }) => Promise<void>;

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
  paste: (sectionIndex: number) => void;
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
        // The view is deliberately kept: regenerating while zoomed in should
        // not throw the producer back to the top of the song.
      });
    } catch (error) {
      set({ generating: false, error: reason(error) });
    }
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
      set({ selection: [clip] });
      return;
    }
    // Additive click toggles, so a mis-shift-click is undone by repeating it
    // rather than by starting the selection over.
    set({
      selection: isSelected(selection, clip)
        ? selection.filter((s) => !sameClip(s, clip))
        : [...selection, clip],
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
    set({ selection: additive ? [...get().selection, ...clips] : clips });
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

  paste(sectionIndex) {
    const clipboard = get().clipboard;
    if (!clipboard) return;
    apply(set, get, (song) => pasteClips(song, clipboard, sectionIndex));
  },
}));

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
}
