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
import { useUi } from './ui';
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

  /**
   * The section playing on repeat, or `null` for the whole record (TASK-072).
   *
   * An index rather than a tick span, because the span is derived from the
   * arrangement and the arrangement moves: resizing an earlier section shifts
   * every bar after it, and a stored span would go on looping the bars the
   * section used to occupy.
   */
  loopSection: number | null;
  /**
   * Parts silenced while auditioning, and parts soloed.
   *
   * ⚠ **A preview control, not an edit.** The song is unchanged and so is the
   * export — the same distinction the per-lane audio mute already draws and
   * labels *preview* on screen. A row mute that quietly changed the exported
   * file would be a much worse control than one that does not.
   */
  mutedParts: Part[];
  soloParts: Part[];

  /**
   * The authored song form the next generation should use (TASK-070).
   *
   * `null` is "the artist chooses" — sampled from the weights the model
   * authored, which is the same meaning absence carries for every pin in this
   * app and what makes two generations of one artist differ.
   *
   * ⚠ Kept across a *generation*, unlike the locks and the loop: it is an
   * instruction about what to build next, so clearing it would mean re-picking
   * the form for every reroll. ⛔ But **not** across an artist change — a form
   * index means a different form for a different artist, and one past the end
   * of what they author makes every Generate fail with no control on screen to
   * clear it. Same rule as `mood`, for the same reason.
   */
  structure: number | null;
  setStructure: (index: number | null) => void;

  /**
   * The clip being auditioned on its own, or `null` (TASK-071).
   *
   * ⛔ **A visible state, not a momentary side effect of clicking.** The
   * roadmap asks for "click cell = solo audition", and a bare click already
   * selects — which is TASK-063B's gesture and cannot be taken away. Worse,
   * arming one looping clip *invisibly* would leave the transport playing
   * something the timeline's own loop and solo badges say it is not, which is
   * the readout-that-lies failure this project has a rule about. So audition is
   * its own control on the clip, it shows while it is on, and it says how to
   * get back to the record.
   */
  audition: ClipId | null;
  auditionClip: (clip: ClipId) => void;
  stopAudition: () => void;

  /**
   * The clip open in a part editor, or `null` (TASK-071's drill-in).
   *
   * Held as the *pattern id* rather than the cell, because that is what an edit
   * has to be written back to — and a resize or a clone can move the cell out
   * from under an editor that is still open.
   */
  drillPatternId: string | null;
  /**
   * The song the drilled-in clip came out of.
   *
   * ⛔ Held beside the id because the id does not identify a song:
   * `pattern_id` is `{model}-{section}-{part}` and carries **no seed**, so two
   * generations of one artist reuse it. `Song.id` does carry the seed.
   */
  drillSongId: string | null;
  drillInto: (clip: ClipId) => void;
  closeDrill: () => void;

  /**
   * What the last export did, for the chip to say (TASK-073).
   *
   * ⛔ Held here rather than shown as a transient toast: a native Save As can
   * sit open for a minute while somebody makes a folder, and the one thing a
   * producer needs afterwards is *where the file went*. A message that has
   * already faded by the time they look back is no message at all.
   */
  exportState: 'idle' | 'running' | 'done' | 'cancelled' | 'failed';
  exportMessage: string | null;
  exportSong: () => Promise<void>;
  /**
   * One file per part, into a folder (TASK-069).
   *
   * ⚠ **MIDI stems, not audio stems.** The preview kit is a drum kit, so the
   * four melodic parts have no voice to render through yet — `export.rs` has
   * the full reasoning and points at FMM-N15/FMM-N16. Four silent wavs called
   * stems would be worse than not offering them.
   */
  exportStems: () => Promise<void>;
  /** Shared by both exports — see the note on the implementation. */
  runExport: (command: 'export_song' | 'export_stems') => Promise<void>;

  /** Hand the arrangement to the audio thread, as it currently stands. */
  armSong: () => void;
  setLoopSection: (index: number | null) => void;
  togglePartMute: (part: Part) => void;
  togglePartSolo: (part: Part) => void;

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

/** What `export_status` answers with. Mirrors `export::Status` in the plugin. */
type ExportStatus =
  | { state: 'idle' }
  | { state: 'running' }
  | { state: 'done'; path: string }
  | { state: 'cancelled' }
  | { state: 'failed'; reason: string };

/** How often the export poll asks. Slow: a human is browsing for a folder. */
const EXPORT_POLL_MS = 400;

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
  loopSection: null,
  mutedParts: [],
  soloParts: [],
  structure: null,
  audition: null,
  drillPatternId: null,
  drillSongId: null,
  exportState: 'idle',
  exportMessage: null,

  async exportSong() {
    await get().runExport('export_song');
  },

  async exportStems() {
    await get().runExport('export_stems');
  },

  // ⛔ One implementation for both, because the *only* difference is the
  // command name. The polling, the timeout and the status mapping were the
  // fiddly parts, and two copies of them is two places for the export to end up
  // stuck reading "exporting…" forever.
  async runExport(command) {
    const { song, exportState } = get();
    if (!song || exportState === 'running') return;
    set({ exportState: 'running', exportMessage: null });
    try {
      await invoke(command, { song });
    } catch (error) {
      // ⛔ **A refusal because one is already open is adopted, not reported.**
      // The plugin keeps one export slot and only this poll ever drains it, so
      // if the page stopped polling — a reloaded webview, an editor window torn
      // down and reopened — the producer met "an export is already open" with
      // no dialog anywhere on screen and no way back. Falling through to the
      // poll picks the in-flight one up and reports where its file went.
      if (!reason(error).includes('already open')) {
        set({ exportState: 'failed', exportMessage: reason(error) });
        return;
      }
    }

    // ⛔ **Polled, because the dialog is modal on its own thread and there is
    // no event to wait on.** `export.rs` explains why the command cannot block:
    // it answers on the frame the page is waiting on, which inside a host is
    // the DAW's editor thread.
    const tick = async (): Promise<void> => {
      let status: ExportStatus;
      try {
        status = await invoke<ExportStatus>('export_status');
      } catch (error) {
        set({ exportState: 'failed', exportMessage: reason(error) });
        return;
      }
      if (status.state !== 'running') {
        set({
          exportState: status.state,
          exportMessage:
            status.state === 'done'
              ? status.path
              : status.state === 'failed'
                ? status.reason
                : null,
        });
        return;
      }
      // ⛔ **No ceiling, and removing it was the fix.** The plugin's dialog
      // thread always publishes a terminal status — `run_dialog` has no early
      // return — so a `running` reply genuinely means a dialog is open, however
      // long a producer spends making and renaming a folder. Giving up after
      // five minutes set the chip idle while the slot stayed claimed: the next
      // Export was refused with no dialog on screen, and when the original was
      // finally confirmed the file was written and its path sat unread until
      // the next claim overwrote it — so the one thing the chip exists to say
      // was never said.
      setTimeout(() => void tick(), EXPORT_POLL_MS);
    };
    setTimeout(() => void tick(), EXPORT_POLL_MS);
  },

  setStructure(index) {
    set({ structure: index });
  },

  auditionClip(clip) {
    // Clicking the control again ends it, so the gesture is its own way out.
    const { audition } = get();
    set({ audition: audition && sameClip(audition, clip) ? null : clip });
    get().armSong();
  },

  stopAudition() {
    if (get().audition === null) return;
    set({ audition: null });
    get().armSong();
  },

  drillInto(clip) {
    const { song } = get();
    const reference = song?.sections[clip.sectionIndex]?.patterns[clip.part];
    const pattern = reference ? song?.patterns[reference.patternId] : undefined;
    if (!reference || !pattern) return;
    set({ drillPatternId: reference.patternId, drillSongId: song.id });
    // ⛔ Straight into the session's own clip slot, so the editors need no
    // second source. They draw `useSession.pattern` and nothing else — an
    // "embed mode" reading from the song would be a second renderer for the
    // same notes, and the two would disagree the first time either changed.
    useSession.getState().openClip(pattern, clip.part);
  },

  closeDrill() {
    set({ drillPatternId: null, drillSongId: null });
  },

  armSong() {
    const { song, loopSection, mutedParts, soloParts, audition } = get();
    if (!song) return;
    // ⛔ **Only while the Song tab is the one on screen.** There is a single
    // schedule and the visible tab decides whose it is — the timeline's mount
    // effect and `armCurrentPattern` exist to keep that true. Undo runs on
    // every tab now, and a project restore runs on whichever tab is open, so
    // without this an undo taken over the drum grid put the whole 56-bar record
    // on the transport while the grid drew four bars.
    if (useUi.getState().activeTab !== 'song') return;
    void invoke('arm_song', {
      request: {
        song,
        // ⚠ Out of range is "no loop" on the far side rather than an error: a
        // stale index arriving after a section was deleted must not stop
        // playback.
        // An audition overrides both the loop and the part filter for as long
        // as it is on — that is what "solo audition of this cell" means, and
        // the timeline says so while it lasts.
        loopSection: audition ? audition.sectionIndex : loopSection,
        parts: audition ? [audition.part] : playingParts(song, mutedParts, soloParts),
      },
      // The reply is the flattened clip and `editor.rs` arms it from its shape;
      // there is nothing for the page to do with it. A rejection means the song
      // was refused at the bridge, and the timeline is still showing what was
      // playing before — so this is deliberately not surfaced as a song error.
    }).catch(() => {});
  },

  setLoopSection(index) {
    set({ loopSection: index });
    get().armSong();
  },

  togglePartMute(part) {
    const { mutedParts } = get();
    set({
      mutedParts: mutedParts.includes(part)
        ? mutedParts.filter((p) => p !== part)
        : [...mutedParts, part],
    });
    get().armSong();
  },

  togglePartSolo(part) {
    const { soloParts } = get();
    set({
      soloParts: soloParts.includes(part)
        ? soloParts.filter((p) => p !== part)
        : [...soloParts, part],
    });
    get().armSong();
  },

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
          structure: get().structure,
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
        // ⛔ The loop names a section by index, and a fresh song has different
        // sections at those indices — a kept loop would repeat whichever bars
        // happened to land there.
        loopSection: null,
        // ⛔ The audition names a section index *and* overrides both the loop
        // and the part filter, so a kept one armed the brand-new song looping
        // one cell of a section the producer never touched.
        audition: null,
        // ⛔ The clipboard holds *pattern ids*, and a pattern id carries no seed
        // (see the drill note below) — so the new song has a clip under the
        // same name and the paste guard passes. With `anchor` cleared it would
        // land on section 0: a clip the producer never copied, on a section they
        // never targeted.
        clipboard: null,
        // ⛔ A solo on a part the new form does not play arms an empty clip and
        // plays silence over a timeline visibly full of them — and `partsInUse`
        // does not draw that row, so there is no lit badge on screen to turn
        // off. The artist-change subscriber already clears these two for this
        // reason; this reset path was not given the same treatment.
        mutedParts: [],
        soloParts: [],
        // ⛔ The drill-in names a clip id that carries no seed, so it resolves
        // against this new song just as happily as the old one — and the next
        // note edited on the part tab wrote the *previous* song's clip into it.
        drillPatternId: null,
        drillSongId: null,
        // The view is deliberately kept: regenerating while zoomed in should
        // not throw the producer back to the top of the song.
      });
      // A fresh song is describable by its seed again, so nothing has to be
      // stored — but the *previous* one may have been edited and saved, and
      // leaving that in the project file would reopen it over this one.
      noteDocumentChange();
      get().armSong();
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
          // Only swing and half-time are read on the far side — everything
          // else a session pins is already carried by the song itself.
          session: useSession.getState().pins,
        },
      });
      // ⛔ **The clipboard and the drill-in are dropped, because the engine
      // prunes.** `reroll_section` mints fresh clip ids and `prune_patterns`
      // deletes every clip no section names — which is exactly what a *cut*
      // clip is, and what a drilled-in clip becomes when its section is
      // re-rolled. Left in place, Ctrl+V after a re-roll pasted nothing and said
      // nothing, and a note edited afterwards on the part tab was written back
      // under an id no section referenced: inaudible, invisible on the timeline,
      // and persisted into the project file as an orphan.
      //
      // ⛔ **The anchor is kept.** Every keyboard gesture in the timeline reads
      // `anchor ?? 0`, so clearing it made the *second* press of `R` re-roll the
      // intro instead of the section the producer was working on — and the next
      // Ctrl+D and Ctrl+V landed there too. The selection goes because the clips
      // it named were replaced; the section it was in did not move.
      set({
        song: next,
        generating: false,
        selection: [],
        clipboard: null,
        drillPatternId: null,
        drillSongId: null,
      });
      // ⛔ A re-rolled section is no longer what the song's own seed produces,
      // so from here the arrangement only exists if it is saved. This is the
      // edit that is easiest to lose, because nothing about it *looks* like an
      // edit — the timeline redraws and the geometry is unchanged.
      markEdited();
    } catch (error) {
      set({ generating: false, error: reason(error) });
    }
  },

  // ⛔ **All three are the same all-or-nothing toggle over a key list**, and the
  // "locked only when *every* one is" rule is stated once in `toggleLocks`. It
  // was written out three times, and a fourth lock gesture would have copied it
  // a fourth — while the badge in the view derived the same rule a fifth.
  toggleLock(clip) {
    toggleLocks(set, get, [lockKey(clip)]);
  },

  toggleSectionLock(index) {
    const section = get().song?.sections[index];
    if (!section) return;
    toggleLocks(
      set,
      get,
      (Object.keys(section.patterns) as Part[]).map((part) =>
        lockKey({ sectionIndex: index, part }),
      ),
    );
  },

  toggleRowLock(part) {
    const { song } = get();
    if (!song) return;
    toggleLocks(
      set,
      get,
      song.sections
        .map((section, index) =>
          section.patterns[part] ? lockKey({ sectionIndex: index, part }) : null,
        )
        .filter((key): key is string => key !== null),
    );
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
    //
    // ⛔ **And so is everything else keyed by section index.** `locks`,
    // `loopSection` and `audition` all name a section by number, and the insert
    // renumbers every section after it — so a lock placed on the chorus drew on
    // the pre-chorus afterwards, and pressing `R` on the chorus sent an empty
    // locked list and regenerated the very clips the padlock said were pinned.
    // The loop brace and a running audition moved the same way. They are
    // *shifted* rather than dropped, because the producer's intent survives the
    // insert — the section they pinned is still there, one place along.
    const before = get();
    apply(set, get, (song) => cloneSection(song, index), true);
    if (get().song === before.song) return;
    set(shiftAfter(before, index));
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
      loopSection: null,
      // The audition filters go with the song too: a producer who soloed the
      // drums on one artist's arrangement has not asked for the next artist's
      // to open with everything else silent.
      mutedParts: [],
      soloParts: [],
      audition: null,
      drillPatternId: null,
      drillSongId: null,
      // ⛔ **The form pin goes with the artist, exactly as `mood` does** — and
      // `session.ts` gives the reason for `mood` in the same words: on a style
      // that offers fewer forms the control is not even rendered, so there is
      // no way on screen to clear it. Kept, a form index pinned on one artist
      // silently built a *different* artist's form 1 — and if the new artist
      // authored fewer forms, every Generate failed with "there is no form 1",
      // leaving Song Mode unusable: the picker lives inside the timeline, which
      // only mounts once a song exists, and no song could be built.
      structure: null,
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
  ({ song, edited }) => {
    useSong.setState({
      song,
      edited,
      // A restored arrangement is not one anybody is mid-gesture on.
      selection: [],
      anchor: null,
      clipboard: null,
      locks: [],
      // ⛔ The loop names a section by index, and an undo can move or remove
      // the section that index pointed at — so a kept loop would repeat bars
      // the producer never chose. The audition filters are kept: they are about
      // *parts*, which an arrangement edit does not renumber.
      loopSection: null,
      error: null,
    });
    // ⛔ Undo has to reach the audio thread, exactly as `session.ts` documents
    // for the clip: without this the producer steps an edit back, watches the
    // timeline change, presses play and hears the arrangement they just undid.
    useSong.getState().armSong();
  },
);

/**
 * Write a drilled-in edit back into the arrangement (TASK-071).
 *
 * ⛔ **A subscriber, not a line in `editPattern`.** `session.ts` documents why
 * for the audio thread — there are four writers of `pattern` and the ritual was
 * forgotten at one of them — and the same argument holds twice over here,
 * because undo is a fifth writer. The roadmap's requirement is that "edits write
 * back to the song", and an opt-in would be a line to remember in every future
 * gesture the roll gains.
 *
 * ⚠ **The clip is shared, and the write-back respects that.** Verse 1 and verse
 * 2 play one entry by id — that is the sharing rule `arrange.rs` states — so
 * editing the verse melody changes it everywhere the verse plays. That is the
 * rule rather than an oversight, and it is what the drill-in banner reports.
 */
useSession.subscribe((state, previous) => {
  if (state.pattern === previous.pattern) return;
  const { song, drillPatternId, drillSongId } = useSong.getState();
  if (!song || drillPatternId === null || state.pattern === null) return;

  // Only while the edited clip is still the one that was drilled into. Pressing
  // Generate on the part tab replaces it with a fresh four-bar loop, which is a
  // new clip rather than an edit of the song's — writing that back would drop a
  // whole section's arrangement into the timeline without anybody asking.
  if (state.pattern.id !== drillPatternId) return;

  // ⛔ **And only into the song it came out of.** `pattern_id` is
  // `{model}-{section}-{part}` — **the seed is not in it** — so two generations
  // of one artist reuse the same clip ids, and the id alone does not say *which
  // song* the clip in hand belongs to. Drill in, press Generate again, then edit
  // a note on the part tab, and song #1's clip silently replaced song #2's in
  // every section that plays it. `Song.id` carries the seed, so it does.

  // ⛔ **And only into the song it came out of.** `pattern_id` is
  // `{model}-{section}-{part}` — **the seed is not in it** — so two generations
  // of one artist reuse the same clip ids, and the id alone does not say *which
  // song* the clip in hand belongs to. Drill in, press Generate again, then edit
  // a note on the part tab, and song #1's clip silently replaced song #2's in
  // every section that plays it. `Song.id` carries the seed, so it does.
  if (song.id !== drillSongId) return;

  // Nothing to write back: `drillInto` hands the editor the *same object* that
  // is already in the store, so a double-click that only looked at a clip would
  // otherwise rebuild the song into an equal-but-new object, mark it edited and
  // push an undo step that changes nothing on screen.
  if (song.patterns[drillPatternId] === state.pattern) return;

  useSong.setState({
    song: { ...song, patterns: { ...song.patterns, [drillPatternId]: state.pattern } },
  });
  // ⛔ **Recorded and saved, but *not* re-armed.** This runs on the part tab
  // with the roll on screen, and the clip has already reached the audio thread
  // through `session.ts`'s own pattern subscriber. Arming the whole song here
  // put the 56-bar record on the transport while the roll drew four bars — the
  // exact readout-that-lies failure `SongTimeline`'s mount effect exists to
  // prevent — and it happened again on every note edit. The timeline arms the
  // arrangement when the producer goes back to it.
  markEdited({ arm: false });
});

/**
 * The parts that should sound, or `null` for all of them.
 *
 * ⛔ **Solo wins over mute, which is what every DAW does and is not merely a
 * convention.** A producer soloing the drums to check a transition has usually
 * muted something earlier and forgotten; making them undo that first would mean
 * solo sometimes did nothing at all, with the row lit up saying otherwise.
 *
 * `null` rather than "every part" so the common case sends no filter and the
 * engine takes the whole-song path — one less place for the list to be wrong.
 */
function playingParts(song: Song, muted: Part[], solo: Part[]): Part[] | null {
  if (solo.length > 0) return solo;
  if (muted.length === 0) return null;
  const all = new Set<Part>();
  for (const section of song.sections) {
    for (const part of Object.keys(section.patterns) as Part[]) all.add(part);
  }
  return [...all].filter((part) => !muted.includes(part));
}

/**
 * The key one clip is locked under.
 *
 * ⛔ Exported, because the view has to ask the same question the store answers.
 * When both hand-rolled the format the string was written in five places across
 * two files and had already drifted — the badge guarded an empty section and the
 * store did not, so a padlock could say one thing while the re-roll it is
 * supposed to describe did another.
 */
export function lockKey({ sectionIndex, part }: ClipId): string {
  return `${sectionIndex}:${part}`;
}

/**
 * Move every index-keyed piece of state past `index` along by one.
 *
 * ⛔ **`locks`, `loopSection` and `audition` all name a section by number**, and
 * `cloneSection` splices a copy in at `index + 1` — so without this a lock
 * placed on the chorus drew on the pre-chorus afterwards, and a re-roll of the
 * chorus regenerated the clips the padlock said were pinned. Shifted rather
 * than dropped: the section the producer pinned still exists, one place along.
 *
 * ⚠ The *cloned* section deliberately does not inherit the source's locks. A
 * copy is a new section nobody has pinned anything on, and inheriting would
 * mean cloning a locked section produced two sections nobody could re-roll.
 */
function shiftAfter(before: SongState, index: number): Partial<SongState> {
  const move = (at: number) => (at > index ? at + 1 : at);
  return {
    locks: before.locks.map((lock) => {
      const [at, part] = lock.split(':');
      return `${move(Number(at))}:${part}`;
    }),
    loopSection: before.loopSection === null ? null : move(before.loopSection),
    audition:
      before.audition === null
        ? null
        : { ...before.audition, sectionIndex: move(before.audition.sectionIndex) },
  };
}

/**
 * Lock every key, or unlock them all if every one is already locked.
 *
 * ⛔ The all-or-nothing rule lives here and nowhere else: a half-locked
 * selection that reported itself locked would let a re-roll change part of it
 * while the badge said otherwise, and the producer would only find out by
 * hearing a section they had pinned come back different.
 */
function toggleLocks(
  set: (partial: Partial<SongState>) => void,
  get: () => SongState,
  keys: string[],
): void {
  if (keys.length === 0) return;
  const { locks } = get();
  const rest = locks.filter((lock) => !keys.includes(lock));
  set({ locks: keys.every((key) => locks.includes(key)) ? rest : [...rest, ...keys] });
}

/**
 * Which sections and which rows are *fully* locked.
 *
 * ⛔ **Every, not any.** A half-locked section reporting itself locked would let
 * a re-roll change part of it while the badge said otherwise — and the producer
 * would only find out by hearing a section they had pinned come back different.
 *
 * Computed once per lock change rather than per render: the view asks for each
 * value three times (class, `aria-pressed`, icon) and re-renders whenever the
 * arrangement does.
 */
export function lockedRegions(
  song: Song,
  locks: string[],
): { sections: boolean[]; rows: Partial<Record<Part, boolean>> } {
  const held = new Set(locks);
  const sections = song.sections.map((section, index) => {
    const parts = Object.keys(section.patterns) as Part[];
    return (
      parts.length > 0 &&
      parts.every((part) => held.has(lockKey({ sectionIndex: index, part })))
    );
  });

  const rows: Partial<Record<Part, boolean>> = {};
  for (const [index, section] of song.sections.entries()) {
    for (const part of Object.keys(section.patterns) as Part[]) {
      const locked = held.has(lockKey({ sectionIndex: index, part }));
      rows[part] = (rows[part] ?? true) && locked;
    }
  }
  return { sections, rows };
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
function markEdited({ arm = true }: { arm?: boolean } = {}): void {
  useSong.setState({ edited: true });
  noteDocumentChange();
  // ⚠ The one caller that passes `false` is the drill-in write-back, which runs
  // while a *part editor* is on screen — see the note there.
  if (arm) useSong.getState().armSong();
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
  set({ song: next, ...(clearSelection ? { selection: [] } : {}) });
  // ⛔ **Through `markEdited`, not its body again.** Its own comment says the
  // flag and the save must not be reachable separately, and re-arming is the
  // third half of that invariant: a resize retiles the whole song, so the clip
  // already on the audio thread describes bars that have moved and the producer
  // would go on hearing the arrangement they had before. Two doors onto an
  // invariant documented as having one is how a third one gets built.
  markEdited();
}
