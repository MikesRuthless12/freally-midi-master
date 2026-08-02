import { useEffect, useRef } from 'react';

import type { Lane, Note, Pattern } from '../../lib/ipc-types';
import { isTypingTarget } from '../../lib/keyboard';
import { useEditing } from '../../state/editing';
import {
  addNote,
  barTicks,
  deleteNotes,
  duplicateNotes,
  moveNotes,
  noteId,
  notesOf,
  selectionLength,
  stepTicks,
  type NoteId,
} from './notes';
import { reselect, stretch } from './transforms';

/**
 * The piano roll's keyboard, to the standard Ableton sets (TASK-041A).
 *
 * ⛔ **A window listener rather than a handler on the canvas, and it is gated on
 * the roll being mounted rather than on it being focused.** A canvas is not
 * focusable without a `tabIndex`, and giving it one puts a focus ring around the
 * whole editor and makes `Tab` land there on the way to the controls beneath.
 * The roll is only ever mounted when a melodic tab is showing its own pattern.
 *
 * ⛔ **Registered once, with the moving parts read through a ref.** The first
 * version listed `pattern` and the playhead in its dependency array — so the
 * listener was torn down and re-added on every note edit *and thirty times a
 * second* while the transport ran, because the playhead poll rewrites it at
 * frame rate. Nothing about the handler changes between those renders.
 *
 * ⛔ **A modal suppresses it.** Settings and About are `document`-level dialogs
 * that do not stop propagation, and the roll stays mounted behind them — so
 * `Delete` under an open Settings pane was deleting notes the user could not
 * see. "Mounted" is not "active"; an open `<dialog>`-role element is the
 * cheapest honest test for the difference until there is a scope stack.
 */

/** The clipboard is module state: it outlives the component across tab switches. */
let clipboard: Note[] = [];

/** Is a modal covering the editor? Then these keys are not ours. */
function modalIsOpen(): boolean {
  return document.querySelector('[role="dialog"]') !== null;
}

export function useRollShortcuts(
  pattern: Pattern,
  lane: Lane,
  editPattern: (next: Pattern) => void,
  /** Where paste lands, in ticks. The roadmap puts it at the playhead. */
  playheadTicks: number,
): void {
  // Everything the handler needs that changes between renders, in one ref, so
  // the listener below never has to be rebuilt. The alternative was rebinding a
  // window listener at 30 Hz.
  //
  // Written in an effect rather than during render: a ref write during render
  // is what `react-hooks/refs` forbids, and rightly — under concurrent
  // rendering a render can be thrown away, which would leave the ref holding
  // state that was never committed.
  const live = useRef({ pattern, lane, editPattern, playheadTicks });
  useEffect(() => {
    live.current = { pattern, lane, editPattern, playheadTicks };
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isTypingTarget(event.target) || modalIsOpen()) return;

      const { pattern, lane, editPattern, playheadTicks } = live.current;
      const editing = useEditing.getState();
      const { selection, snap } = editing;
      const step = stepTicks(snap, pattern.ppq);
      const bar = barTicks(pattern);
      const accel = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();

      // ⛔ Undo and redo are deliberately absent. `App.tsx` already owns Ctrl+Z
      // and Ctrl+Y for the whole app, and a note edit is an ordinary entry on
      // that same stack — a second handler here would either double-undo or
      // quietly shadow the global one.

      if (accel && key === 'a') {
        event.preventDefault();
        editing.select(notesOf(pattern, lane).map(noteId));
        return;
      }

      if (event.key === 'Escape') {
        editing.clearSelection();
        return;
      }

      // ⛔ Paste is checked *before* the empty-selection guard below. Everything
      // after that guard acts on a selection; a paste acts on the clipboard, and
      // requiring something to be selected first would make "copy, click away,
      // paste" — the ordinary gesture — do nothing.
      if (accel && key === 'v') {
        if (clipboard.length === 0) return;
        event.preventDefault();
        // Anchored on the earliest note so the block lands *at* the playhead
        // rather than starting one clipboard-length before it.
        const anchor = Math.min(...clipboard.map((note) => note.startTick));
        const offset = Math.round(playheadTicks) - anchor;
        let next = pattern;
        const pasted: NoteId[] = [];
        for (const note of clipboard) {
          const placed = { ...note, startTick: note.startTick + offset };
          next = addNote(next, lane, placed);
          pasted.push(noteId(placed));
        }
        editPattern(next);
        editing.select(pasted);
        return;
      }

      if (selection.size === 0) return;

      if (event.key === 'Delete' || event.key === 'Backspace') {
        event.preventDefault();
        editPattern(deleteNotes(pattern, lane, selection));
        editing.clearSelection();
        return;
      }

      // Copy and cut fill the clipboard from the notes themselves rather than
      // from ids: an id is only meaningful against the pattern it came from, and
      // a paste may happen after an edit has moved everything.
      if (accel && (key === 'c' || key === 'x')) {
        event.preventDefault();
        clipboard = notesOf(pattern, lane).filter((note) => selection.has(noteId(note)));
        if (key === 'x') {
          editPattern(deleteNotes(pattern, lane, selection));
          editing.clearSelection();
        }
        return;
      }

      // Duplicate. `Shift+D` places the copy one selection-length later, which
      // is Ableton's Duplicate; `Ctrl/Cmd+D` places it in place. One branch,
      // because the offset is the only thing that differs.
      if (key === 'd' && (accel || event.shiftKey)) {
        event.preventDefault();
        const offset = event.shiftKey ? selectionLength(pattern, lane, selection) : 0;
        const result = duplicateNotes(pattern, lane, selection, offset);
        editPattern(result.pattern);
        // Only the spaced copy takes the selection — leaving it on an in-place
        // duplicate would be indistinguishable from having done nothing.
        if (offset > 0 && result.ids.length > 0) editing.select(result.ids);
        return;
      }

      // Stretch and compress (TASK-041D). The roadmap names these two keys, and
      // they are the only transforms with a binding — the rest live in the menu
      // rather than in shortcuts nobody asked for and nobody would find.
      if (event.key === '*' || event.key === '/') {
        event.preventDefault();
        const next = stretch(pattern, lane, selection, event.key === '*' ? 2 : 0.5);
        if (next === pattern) return;
        editPattern(next);
        editing.select(reselect(pattern, next, lane, selection));
        return;
      }

      // Transpose and nudge. Shift widens the unit in both axes — an octave
      // vertically, a bar horizontally — which is the pairing both DAWs use.
      const arrows: Record<string, [number, number]> = {
        ArrowUp: [0, event.shiftKey ? 12 : 1],
        ArrowDown: [0, event.shiftKey ? -12 : -1],
        ArrowLeft: [event.shiftKey ? -bar : -step, 0],
        ArrowRight: [event.shiftKey ? bar : step, 0],
      };
      const delta = arrows[event.key];
      if (delta) {
        event.preventDefault();
        const [dt, dp] = delta;
        const next = moveNotes(pattern, lane, selection, dt, dp);
        editPattern(next);
        // ⛔ The selection is re-derived, because a `NoteId` is start tick and
        // pitch — the two things that just moved. Leaving it alone would point
        // at notes that no longer exist, so the next arrow press would move
        // nothing and the selection would appear to have been lost.
        //
        // `reselect` rather than mapping each note back through the delta: it
        // is the question every transform asks, it is already imported for the
        // stretch above, and a delta-specific second answer only ever works for
        // the one edit that happens to have a delta.
        editing.select(reselect(pattern, next, lane, selection));
      }
    };

    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}
