import type { Lane, Note, Part, Pattern } from '../../lib/ipc-types';

/**
 * The piano roll's arithmetic, kept out of the component (TASK-041).
 *
 * Same split as `DrumGrid/cells.ts` and for the same two reasons: fast refresh
 * only works when a module exports components alone, and this is the half with
 * logic worth testing on its own. A note snapped one tick late is an edit that
 * looks right on screen and exports wrong.
 *
 * ⛔ **Every edit here returns a new `Pattern` and shares everything it did not
 * touch.** `state/history.ts` keeps unlimited undo affordable by holding the
 * pattern *by reference*, so a snapshot costs one pointer rather than a copy of
 * the notes. Rebuilding all the lanes on every edit would quietly turn a
 * hundred-step history into a hundred full patterns — the file says so, and it
 * stops being true the moment notes become editable.
 */

/** MIDI's own range. A pitch outside it cannot be expressed in the file format. */
export const MIN_PITCH = 0;
export const MAX_PITCH = 127;

/** MIDI velocity. Zero is note-off in the wire format, so a note's floor is 1. */
export const MIN_VELOCITY = 1;
export const MAX_VELOCITY = 127;

/**
 * The snap values the grid offers, coarsest first.
 *
 * Triplets are in the same list rather than behind a modifier because they are
 * a *grid*, not a variation on one: a producer working in 12/8 sets it once and
 * every gesture follows. `off` is last and is the only entry that is not a
 * division — it means no quantisation at all, which is what an edit against a
 * humanized performance needs.
 */
export const SNAP_VALUES = [
  '1/4',
  '1/8',
  '1/16',
  '1/32',
  '1/64',
  '1/4T',
  '1/8T',
  '1/16T',
  'off',
] as const;

export type Snap = (typeof SNAP_VALUES)[number];

/** What each snap divides a whole note into. Triplets are three per two. */
const SNAP_DIVISIONS: Record<Exclude<Snap, 'off'>, number> = {
  '1/4': 4,
  '1/8': 8,
  '1/16': 16,
  '1/32': 32,
  '1/64': 64,
  '1/4T': 6,
  '1/8T': 12,
  '1/16T': 24,
};

/**
 * How many ticks one snap step is, or `null` when snapping is off.
 *
 * ⛔ Not rounded. `ppq` is 960 in this engine precisely so the awkward divisions
 * come out whole — a 1/16T is 160 ticks exactly — and rounding here would put a
 * drift of a tick or two into every triplet, which accumulates across a bar into
 * an audible flam. A `ppq` that cannot express the division is a bug in the
 * caller, not something to paper over.
 */
export function snapTicks(snap: Snap, ppq: number): number | null {
  if (snap === 'off') return null;
  return (ppq * 4) / SNAP_DIVISIONS[snap];
}

/** Round a tick to the nearest snap step, clamped to zero. */
export function snapTick(tick: number, snap: Snap, ppq: number): number {
  const step = snapTicks(snap, ppq);
  if (step === null) return Math.max(0, Math.round(tick));
  return Math.max(0, Math.round(tick / step) * step);
}

/**
 * Ticks in the whole clip — what the roll's width represents.
 *
 * ⛔ **Bars × the clip's own bar, not bars × four quarters.** This assumed
 * common time, which was invisible until TASK-041E made the meter settable and
 * is `engine::context::total_ticks`'s own definition — a 6/8 clip whose length
 * the UI computed as four quarters a bar would draw a third more grid than the
 * engine generates, clamp notes against a clip end that is not there, and put
 * the loop brace's far end past the last note.
 */
export function patternTicks(pattern: Pattern): number {
  return Math.max(1, pattern.bars * barTicks(pattern));
}

/**
 * Ticks in one bar, honouring the clip's meter.
 *
 * One definition, because the ruler draws bar lines from it and `Shift`+arrow
 * nudges by it — and a grid that disagrees with the keyboard about where a bar
 * ends is a nudge that visibly lands off the line it aimed at.
 */
export function barTicks(pattern: Pattern): number {
  return Math.max(1, pattern.ppq * pattern.timeSigNum * (4 / pattern.timeSigDen));
}

/**
 * The tick step a gesture moves by, with a defined answer when snapping is off.
 *
 * ⛔ **One fallback, not four.** This expression was written out at four call
 * sites with three *different* answers for `off` — a whole note, one tick, and a
 * quarter twice — so the grid, the resize floor, the drawn note length and the
 * arrow-key nudge each meant something different by "no snap". A sixteenth is
 * the answer: it is the resolution the drum grid is already thought in, and it
 * is a usable default length for a note drawn with the grid switched off.
 */
export function stepTicks(snap: Snap, ppq: number): number {
  return snapTicks(snap, ppq) ?? ppq / 4;
}

/**
 * Which lane a part's notes live in.
 *
 * The four melodic parts each generate exactly one lane and it carries the
 * part's own name, so this is a rename rather than a lookup — but it is written
 * once here so a component never has to assume it.
 *
 * ⛔ Drums is deliberately absent. It has nine lanes and no single pitch axis,
 * which is why it has a grid rather than a roll; asking this for `drums` is a
 * caller that has not decided which surface it is drawing.
 */
export type MelodicPart = Exclude<Part, 'drums'>;

export function laneOf(part: MelodicPart): Lane {
  return part;
}

/**
 * A note's identity, for selection that survives an edit.
 *
 * ⛔ **Value-keyed rather than indexed, because an edit reorders the array.**
 * Dragging a note earlier moves it past its neighbours, so an index captured on
 * pointerdown names a *different* note by pointerup — the selection would
 * silently jump to whatever slid into that slot. Start tick and pitch together
 * are unique within a lane (`no_lane_holds_two_notes_at_one_tick_and_pitch`
 * asserts it across every shipped model), and they are exactly the two fields a
 * move changes, so the key is re-derived after every commit rather than stored.
 */
export type NoteId = string;

/**
 * Which way `Ctrl`+R turns a selection: the one direction it can be un-done from.
 *
 * ⛔⛔ **A TOGGLE DECIDED ONCE FOR THE WHOLE SELECTION.** Flipping each note
 * independently would make a mixed selection un-un-reversible — every press
 * would swap which half was backwards and there would be no way back to "all
 * forwards". So if *anything* selected is still forwards the press reverses
 * everything, and only a wholly-reversed selection turns back.
 *
 * ⛔ **One definition, because two grids implement the gesture.**
 * `PianoRoll/transforms.ts::reversePlayback` works on note ids and
 * `DrumGrid/cells.ts::reverseCells` on cell references — the selection models
 * genuinely differ, the *policy* does not, and both files' docs used to promise
 * the rule was "deliberately identical" while stating it separately.
 *
 * ⚠ **There is no "nothing would change" case to guard.** It was written out in
 * both copies as `if (notes.every(n => n.reversed === backwards)) return`, and
 * it is a contradiction either way round: all-reversed gives `backwards = false`
 * and needs them all false, any-forward gives `true` and needs them all true.
 */
export function nextReversed(notes: readonly Note[]): boolean {
  return !notes.every((note) => note.reversed);
}

export function noteId(note: Note): NoteId {
  return `${note.startTick}:${note.pitch}`;
}

/** The notes of one lane, or an empty list when the pattern has no such lane. */
export function notesOf(pattern: Pattern, lane: Lane): Note[] {
  return pattern.lanes.find((track) => track.lane === lane)?.notes ?? [];
}

/**
 * Replace one lane's notes, sharing every other lane untouched.
 *
 * Notes are kept sorted by start tick, then pitch: the renderer walks them in
 * order to cull what is off-screen, and [`noteId`]'s uniqueness claim is much
 * easier to hold to when a duplicate would be adjacent. A lane the pattern does
 * not have yet is appended, which is what makes drawing the first note on an
 * empty melody clip work.
 */
export function withNotes(pattern: Pattern, lane: Lane, notes: Note[]): Pattern {
  const sorted = [...notes].sort((a, b) => a.startTick - b.startTick || a.pitch - b.pitch);
  const existing = pattern.lanes.some((track) => track.lane === lane);

  return {
    ...pattern,
    lanes: existing
      ? pattern.lanes.map((track) => (track.lane === lane ? { lane, notes: sorted } : track))
      : [...pattern.lanes, { lane, notes: sorted }],
  };
}

/**
 * Drop notes that now sit on top of another, keeping the one that just moved.
 *
 * A move or a transpose can land two notes on one tick and pitch — drag a note
 * onto its neighbour and they occupy the same slot. Two notes there are one
 * audible note and two MIDI events, the second of which cuts the first short,
 * so the pattern would export something nobody drew.
 *
 * The last writer wins, so **every caller must put the notes it just changed at
 * the end** — see `mapNotes`, which is the one that has to work at it.
 */
function dedupe(notes: Note[]): Note[] {
  const byId = new Map<NoteId, Note>();
  for (const note of notes) byId.set(noteId(note), note);
  return [...byId.values()];
}

/** Hold a note inside the clip and inside MIDI's own ranges. */
function clampNote(note: Note, ticks: number): Note {
  const lenTicks = Math.max(1, Math.round(note.lenTicks));
  const startTick = Math.min(Math.max(0, Math.round(note.startTick)), Math.max(0, ticks - 1));
  return {
    ...note,
    startTick,
    // ⛔ Trimmed to the clip end rather than allowed past it. `engine/src/midi.rs`
    // already refuses to write a note that outlives the pattern — the export
    // gate `nothing_sounds_past_the_end_of_the_clip` is what proves it — so a
    // note dragged long would look held on screen and arrive shortened in the
    // DAW, which is the readout-that-lies failure this codebase keeps closing.
    lenTicks: Math.max(1, Math.min(lenTicks, ticks - startTick)),
    pitch: Math.min(MAX_PITCH, Math.max(MIN_PITCH, Math.round(note.pitch))),
    vel: Math.min(MAX_VELOCITY, Math.max(MIN_VELOCITY, Math.round(note.vel))),
  };
}

/** Put one note into a lane. */
export function addNote(pattern: Pattern, lane: Lane, note: Note): Pattern {
  const ticks = patternTicks(pattern);
  return withNotes(pattern, lane, dedupe([...notesOf(pattern, lane), clampNote(note, ticks)]));
}

/** Remove every selected note. */
export function deleteNotes(pattern: Pattern, lane: Lane, ids: ReadonlySet<NoteId>): Pattern {
  if (ids.size === 0) return pattern;
  return withNotes(
    pattern,
    lane,
    notesOf(pattern, lane).filter((note) => !ids.has(noteId(note))),
  );
}

/**
 * Apply a change to the selected notes and leave the rest alone.
 *
 * The one place an edit is expressed, so clamping, deduping and re-sorting
 * cannot be forgotten by a caller that only wanted to nudge a pitch.
 */
export function mapNotes(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  change: (note: Note) => Note,
): Pattern {
  if (ids.size === 0) return pattern;
  const ticks = patternTicks(pattern);

  // ⛔ **The changed notes go last, and that is what decides who survives a
  // collision.** `dedupe` keeps the last writer, and mapping in place left a
  // note dragged *forward* onto a neighbour still sitting earlier in the list —
  // so the note the producer had hold of was the one thrown away, and the
  // stationary one stayed wearing its own length and velocity. Dragging
  // backward worked, which is exactly what made it read as a snapping quirk
  // rather than as a note being deleted. One `ArrowRight` did it too, and then
  // the selection pointed at a note that no longer existed.
  const untouched: Note[] = [];
  const changed: Note[] = [];
  for (const note of notesOf(pattern, lane)) {
    if (ids.has(noteId(note))) changed.push(clampNote(change(note), ticks));
    else untouched.push(note);
  }

  return withNotes(pattern, lane, dedupe([...untouched, ...changed]));
}

/**
 * Move a selection by a tick and pitch delta.
 *
 * ⛔ **The delta is clamped once for the whole selection, not per note.** Nudging
 * a chord down when its lowest note is already at pitch 0 must move nothing —
 * clamping each note on its own instead lets the top notes fall while the bottom
 * one sticks, which silently re-voices the chord. Ableton and FL both hold the
 * shape; so does this.
 */
export function moveNotes(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  deltaTicks: number,
  deltaPitch: number,
  /** Leave the originals where they are and place copies at the delta. */
  copy = false,
): Pattern {
  if (ids.size === 0) return pattern;
  const selected = notesOf(pattern, lane).filter((note) => ids.has(noteId(note)));
  if (selected.length === 0) return pattern;

  const ticks = patternTicks(pattern);
  const { deltaTicks: dt, deltaPitch: dp } = clampedMove(
    pattern,
    lane,
    ids,
    deltaTicks,
    deltaPitch,
  );
  if (dt === 0 && dp === 0) return pattern;

  // ⛔ **`copy` lives here, not in the caller, because the *clamped* delta is
  // here.** Alt-drag has to leave the originals and place the copies where the
  // drag actually ended — and "where it actually ended" is `dt`/`dp` above,
  // clamped once for the whole selection. A caller doing its own duplicate-then-
  // move would either re-derive that clamp or get it wrong at the clip's edges.
  if (copy) {
    const moved = selected.map((note) =>
      clampNote({ ...note, startTick: note.startTick + dt, pitch: note.pitch + dp }, ticks),
    );
    // Copies last: `dedupe` keeps the last writer, and a copy dropped onto an
    // existing note is the producer placing it there.
    return withNotes(pattern, lane, dedupe([...notesOf(pattern, lane), ...moved]));
  }

  return mapNotes(pattern, lane, ids, (note) => ({
    ...note,
    startTick: note.startTick + dt,
    pitch: note.pitch + dp,
  }));
}

/**
 * The delta a move can actually have, clamped once for the whole selection.
 *
 * ⛔ **Exported so the *preview* can use it too.** The renderer drew the raw
 * delta while `moveNotes` committed a clamped one, so dragging a chord toward
 * the clip's end or toward pitch 127 kept sliding on screen after the commit had
 * stopped — and the notes landed where the canvas had stopped showing them. One
 * function, asked by both, is the only arrangement where they cannot disagree.
 */
export function clampedMove(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  deltaTicks: number,
  deltaPitch: number,
): { deltaTicks: number; deltaPitch: number } {
  const selected = notesOf(pattern, lane).filter((note) => ids.has(noteId(note)));
  if (selected.length === 0) return { deltaTicks: 0, deltaPitch: 0 };

  const ticks = patternTicks(pattern);
  const minStart = Math.min(...selected.map((note) => note.startTick));
  const maxEnd = Math.max(...selected.map((note) => note.startTick + note.lenTicks));
  const minPitch = Math.min(...selected.map((note) => note.pitch));
  const maxPitch = Math.max(...selected.map((note) => note.pitch));

  return {
    // ⛔ The headroom is floored at zero, not used raw. `ticks - maxEnd` goes
    // *negative* the moment any selected note ends past the clip — which the
    // meter picker makes reachable, since switching 4/4 to 6/8 shortens the clip
    // under notes that were legal a moment ago. Unfloored, that negative
    // headroom became the delta: a pure pitch nudge, with `deltaTicks` of
    // exactly 0, dragged the whole selection left. Nothing can move is the right
    // answer; moving backwards is not.
    deltaTicks: Math.max(-minStart, Math.min(deltaTicks, Math.max(0, ticks - maxEnd))),
    deltaPitch: Math.max(MIN_PITCH - minPitch, Math.min(deltaPitch, MAX_PITCH - maxPitch)),
  };
}

/** Which edge of a note a resize drag has hold of. */
export type Edge = 'start' | 'end';

/**
 * Resize a selection from one edge by a tick delta.
 *
 * The left edge moves the start and keeps the end; the right edge moves the end
 * and keeps the start (TASK-041A). Clamped for the selection as a whole, for
 * the same reason [`moveNotes`] is — and additionally floored at one snap step
 * so a drag past a note's own start cannot invert it into a negative length.
 */
export function resizeNotes(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  edge: Edge,
  deltaTicks: number,
  minLen: number,
): Pattern {
  if (ids.size === 0) return pattern;
  const selected = notesOf(pattern, lane).filter((note) => ids.has(noteId(note)));
  if (selected.length === 0) return pattern;

  const ticks = patternTicks(pattern);
  const floor = Math.max(1, Math.round(minLen));

  if (edge === 'start') {
    // A start may travel back to zero and forward until the shortest note in the
    // selection is down to the floor.
    const minStart = Math.min(...selected.map((note) => note.startTick));
    const slack = Math.min(...selected.map((note) => note.lenTicks - floor));
    const dt = Math.max(-minStart, Math.min(deltaTicks, slack));
    if (dt === 0) return pattern;
    return mapNotes(pattern, lane, ids, (note) => ({
      ...note,
      startTick: note.startTick + dt,
      lenTicks: note.lenTicks - dt,
    }));
  }

  const maxEnd = Math.max(...selected.map((note) => note.startTick + note.lenTicks));
  const slack = Math.min(...selected.map((note) => note.lenTicks - floor));
  // Floored for the reason `moveNotes` floors its own: a selection already past
  // the clip end has negative headroom, and using it raw would *shorten* every
  // note in the selection on a drag that asked to lengthen them.
  const dt = Math.max(-slack, Math.min(deltaTicks, Math.max(0, ticks - maxEnd)));
  if (dt === 0) return pattern;
  return mapNotes(pattern, lane, ids, (note) => ({ ...note, lenTicks: note.lenTicks + dt }));
}

/**
 * Copy the selection, offset in time, and report what the copies are called.
 *
 * Returns the ids so the caller can leave the *copy* selected — Ableton's
 * Duplicate does, and it is what makes `Shift+D` twice produce three spaced
 * copies rather than three stacked on the second.
 *
 * ⛔ A copy that lands on an existing note replaces it, through the same
 * [`dedupe`] every other edit uses. Silently stacking two notes on one tick is
 * one audible note and two MIDI events.
 */
export function duplicateNotes(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  offsetTicks: number,
): { pattern: Pattern; ids: NoteId[] } {
  const selected = notesOf(pattern, lane).filter((note) => ids.has(noteId(note)));
  if (selected.length === 0) return { pattern, ids: [] };

  const ticks = patternTicks(pattern);
  const copies = selected
    // ⛔ Filtered *before* clamping, on the un-clamped position. Clamping first
    // pins an overshooting copy to the last tick and the check then always
    // passes — so duplicating near the end silently built a stack of notes on
    // the final tick instead of doing nothing.
    .filter((note) => note.startTick + offsetTicks < ticks)
    .map((note) => clampNote({ ...note, startTick: note.startTick + offsetTicks }, ticks));

  if (copies.length === 0) return { pattern, ids: [] };

  return {
    pattern: withNotes(pattern, lane, dedupe([...notesOf(pattern, lane), ...copies])),
    ids: copies.map(noteId),
  };
}

/** How far a selection reaches, for Duplicate's offset. Zero when empty. */
export function selectionLength(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
): number {
  const selected = notesOf(pattern, lane).filter((note) => ids.has(noteId(note)));
  if (selected.length === 0) return 0;
  const from = Math.min(...selected.map((note) => note.startTick));
  const to = Math.max(...selected.map((note) => note.startTick + note.lenTicks));
  return Math.max(1, to - from);
}

/** Every note that falls inside a rubber-band rectangle, in ticks and pitches. */
export function notesInBox(
  pattern: Pattern,
  lane: Lane,
  box: { fromTick: number; toTick: number; fromPitch: number; toPitch: number },
): NoteId[] {
  const lowTick = Math.min(box.fromTick, box.toTick);
  const highTick = Math.max(box.fromTick, box.toTick);
  const lowPitch = Math.min(box.fromPitch, box.toPitch);
  const highPitch = Math.max(box.fromPitch, box.toPitch);

  return notesOf(pattern, lane)
    .filter(
      (note) =>
        note.pitch >= lowPitch &&
        note.pitch <= highPitch &&
        // Touched, not enclosed: a marquee dragged across the middle of a long
        // note must catch it. Requiring containment makes a held note
        // unselectable at any zoom where it is wider than the screen.
        note.startTick + note.lenTicks > lowTick &&
        note.startTick < highTick,
    )
    .map(noteId);
}

/** The note under a point, topmost first — the one a click should take. */
export function noteAt(
  pattern: Pattern,
  lane: Lane,
  tick: number,
  pitch: number,
  /**
   * The smallest span a note is *drawn* over, in ticks.
   *
   * ⛔ The renderer floors a note's width at two pixels so a 1/64 at low zoom is
   * still visible; without the same floor here it was visible over two pixels
   * and clickable over a fraction of one. The caller converts that floor from
   * pixels, because only it knows the zoom.
   */
  minTicks = 0,
): Note | null {
  // Reversed so the last-drawn note wins, matching what the renderer paints on
  // top when two notes overlap in time on the same row.
  const notes = notesOf(pattern, lane);
  for (let index = notes.length - 1; index >= 0; index -= 1) {
    const note = notes[index];
    if (note.pitch !== pitch) continue;
    const span = Math.max(note.lenTicks, minTicks);
    if (tick >= note.startTick && tick < note.startTick + span) return note;
  }
  return null;
}

/**
 * The ids a lane gained between two patterns.
 *
 * ⛔ **Because  clamps, and a  is the two fields it clamps.**
 * Both callers built the id from the note they *asked* for: double-clicking in
 * the dimmed space past the clip end drew a note that was then not selected,
 * and pasting a block that overran the end clamped every overrunning note onto
 * the last tick — where  kept one and the rest vanished, with the
 * reported ids naming notes that were never there. Asking the resulting pattern
 * is the only answer that cannot be wrong.
 */
export function addedIds(before: Pattern, after: Pattern, lane: Lane): NoteId[] {
  const had = new Set(notesOf(before, lane).map(noteId));
  return notesOf(after, lane)
    .map(noteId)
    .filter((id) => !had.has(id));
}
