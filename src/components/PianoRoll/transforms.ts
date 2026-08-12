/**
 * The transforms Ableton and FL put a menu on (TASK-041D).
 *
 * Pure functions from a pattern to a pattern, so every one of them is a single
 * `editPattern` call and therefore a single undo step (TASK-042). Nothing here
 * touches the store, the canvas or the clock — a transform that reached for any
 * of those could not be asserted note by note, which is what the roadmap asks
 * these be verified with.
 *
 * ⛔ **Every one goes through `mapNotes` or `withNotes`**, which own the
 * clamping, the dedupe and the re-sort. A transform that built its own note
 * array would be the one place a note could end up outside the clip, at pitch
 * 130, or stacked on a neighbour it silently cuts short.
 */

import type { Lane, Note, Pattern } from '../../lib/ipc-types';
import {
  MAX_VELOCITY,
  MIN_VELOCITY,
  deleteNotes,
  mapNotes,
  nextReversed,
  noteId,
  notesOf,
  patternTicks,
  type NoteId,
} from './notes';

/** The selected notes, in time order. Empty when nothing is selected. */
function selected(pattern: Pattern, lane: Lane, ids: ReadonlySet<NoteId>): Note[] {
  return notesOf(pattern, lane)
    .filter((note) => ids.has(noteId(note)))
    .sort((a, b) => a.startTick - b.startTick || a.pitch - b.pitch);
}

/** The span a selection covers, from its first start to its last end. */
function span(notes: readonly Note[]): { from: number; to: number } {
  return {
    from: Math.min(...notes.map((note) => note.startTick)),
    to: Math.max(...notes.map((note) => note.startTick + note.lenTicks)),
  };
}

/**
 * Chord inversion: the lowest note up an octave, or the highest down.
 *
 * ⛔ **One note moves, not all of them.** That is what makes it an inversion
 * rather than a transpose — pressing it repeatedly walks a triad through its
 * inversions, which is the whole reason the gesture exists. Moving every note
 * by an octave changes nothing anyone can hear about the voicing.
 */
export function invert(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  direction: 'up' | 'down' = 'up',
): Pattern {
  const notes = selected(pattern, lane, ids);
  if (notes.length === 0) return pattern;

  const pitches = notes.map((note) => note.pitch);
  const edge = direction === 'up' ? Math.min(...pitches) : Math.max(...pitches);
  const moved = direction === 'up' ? edge + 12 : edge - 12;
  if (moved < 0 || moved > 127) return pattern;

  // The *first* note on that pitch, so a doubled root inverts one voice at a
  // time rather than teleporting both and leaving the chord a note short.
  const target = notes.find((note) => note.pitch === edge);
  if (target === undefined) return pattern;
  const id = noteId(target);

  return mapNotes(pattern, lane, new Set([id]), (note) => ({ ...note, pitch: moved }));
}

/**
 * Mirror the selection in time.
 *
 * Reflected about the selection's own span rather than the clip's, so reversing
 * two bars of a four-bar clip leaves the other two where they were.
 *
 * ⛔⛔ **NOTES THAT SOUNDED TOGETHER STILL SOUND TOGETHER** — Mike, 2026-08-11:
 * *"if you reverse two countermelody lines that are supposed to be played at the
 * same time, then they should play at the same time, not back to back."*
 *
 * ▶ **It reflected each note's own end**, so two voices sharing an onset but not
 * a length ended at different ticks and came back *starting* at different ticks
 * — a two-part counterline turned into one part answering the other.
 *
 * ⛔⛔ **THE FIX IS TO REVERSE THE *GAPS*, NOT TO REFLECT EACH ONSET.** The
 * obvious repair — reflect by the group's *end* instead of the note's — keeps a
 * chord together but is **not injective**: two groups whose ends happen to
 * coincide map to the same tick, and `mapNotes` runs its result through
 * `dedupe`, which keeps one note per `startTick:pitch` and silently drops the
 * rest. A C4 at tick 0 under a longer G4, plus a C4 at 240, both land on one
 * tick and the clip comes back a note short with nothing to say so.
 *
 * ▶ **So this walks the distinct onsets from the back**, laying each one down
 * after the *gap that used to follow the one behind it*. Every gap is strictly
 * positive, so distinct onsets stay distinct; a whole chord moves as one, so
 * simultaneity is preserved; and for a single-voice line it is identical to the
 * old reflection, which is why the golden test above did not have to change.
 *
 * ⚠ Each note keeps its own length. Only the onsets move.
 */
export function reverse(pattern: Pattern, lane: Lane, ids: ReadonlySet<NoteId>): Pattern {
  const notes = selected(pattern, lane, ids);
  if (notes.length === 0) return pattern;
  const { from, to } = span(notes);

  const onsets = [...new Set(notes.map((note) => note.startTick))].sort((a, b) => a - b);
  // What follows each onset: the next one, or the end of the span for the last.
  const gap = onsets.map((at, i) => (i + 1 < onsets.length ? onsets[i + 1] : to) - at);

  const moved = new Map<number, number>();
  let cursor = from;
  for (let i = onsets.length - 1; i >= 0; i--) {
    moved.set(onsets[i], cursor);
    cursor += gap[i];
  }

  return mapNotes(pattern, lane, ids, (note) => ({
    ...note,
    startTick: moved.get(note.startTick) ?? note.startTick,
  }));
}

/**
 * Scale the selection's offsets and durations by a factor.
 *
 * ⛔ **Anchored at the selection's first note, not at tick zero.** Scaling from
 * the origin drags a selection in bar 4 halfway across the clip, which is a move
 * the producer did not ask for on top of the stretch they did.
 */
export function stretch(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  factor: number,
): Pattern {
  const notes = selected(pattern, lane, ids);
  if (notes.length === 0 || !(factor > 0)) return pattern;
  const { from } = span(notes);
  const at = (note: Note) => from + (note.startTick - from) * factor;

  // ⛔ **A note stretched past the clip's end is removed, not pinned there.**
  // `clampNote` holds a note inside the clip by moving it to the last tick — so
  // a ×2 stretch of a full bar would pile every overflowing note on one tick,
  // where `dedupe` keeps the last and the rest vanish without explanation. One
  // note leaving the clip is a thing a producer can see and undo; six notes
  // silently collapsing into one is not.
  const ticks = patternTicks(pattern);
  const overflow = new Set(notes.filter((note) => at(note) >= ticks).map(noteId));
  const trimmed = overflow.size > 0 ? deleteNotes(pattern, lane, overflow) : pattern;
  const kept = new Set([...ids].filter((id) => !overflow.has(id)));

  return mapNotes(trimmed, lane, kept, (note) => ({
    ...note,
    startTick: at(note),
    lenTicks: Math.max(1, note.lenTicks * factor),
  }));
}

/**
 * Play the selected notes' samples backwards — or forwards again.
 *
 * ⛔ **NOT [`reverse`] above, and the two are easy to confuse.** That one is a
 * *retrograde*: it flips the notes' order in time, so the last hit plays first.
 * This changes no tick at all — it plays each note's own sample back to front,
 * which is a sound-design gesture rather than a compositional one. Naming it
 * `reverseNotes` beside a `reverse` that reverses notes was a collision waiting
 * to be reached for by the wrong name.
 *
 * ⛔⛔ **Mike, 2026-08-11:** *"if you have a drum pad or something playing forward
 * in the pad, but you want it to play backwards in the drum pattern, you should
 * be able to switch it, the same with a melody/chords/bassline/counter melody
 * within the actual note itself — you should be able to select the note and press
 * like 'Ctrl+R' or 'Command+R' on macOS to reverse the note just for that single
 * note being played. i think this would be a VERY USABLE AND COOL FEATURE to
 * have."*
 *
 * ⛔ **A TOGGLE, decided once for the whole selection — see [`nextReversed`],**
 * which is where that rule lives so the drum grid's `reverseCells` cannot come
 * to a different answer about what a mixed selection does.
 *
 * ⚠ **Nothing moves.** Unlike every other transform here this changes no tick
 * and no pitch, so the selection stays valid and the caller does not need
 * `reselect` — which is worth knowing, because passing through it would be
 * harmless and would look like the note ids had changed.
 *
 * ⛔ **Audio only.** A reversed note has no SMF representation, so a `.mid`
 * dragged or exported from this clip loses it — `engine::pattern::Note::reversed`
 * carries the same warning on the other side of the bridge.
 */
export function reversePlayback(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
): Pattern {
  const notes = selected(pattern, lane, ids);
  if (notes.length === 0) return pattern;
  const backwards = nextReversed(notes);
  return mapNotes(pattern, lane, ids, (note) => ({ ...note, reversed: backwards }));
}

/**
 * Extend every note to where the next one starts.
 *
 * ⛔ **"The next" is the next *onset*, not the next note in the list.** A chord
 * is several notes on one tick, and stretching each to the next entry would
 * leave the members of a triad at three different lengths — one of them zero.
 * The last onset keeps its length: there is nothing after it to reach.
 */
export function legato(pattern: Pattern, lane: Lane, ids: ReadonlySet<NoteId>): Pattern {
  const notes = selected(pattern, lane, ids);
  if (notes.length === 0) return pattern;

  const onsets = [...new Set(notes.map((note) => note.startTick))].sort((a, b) => a - b);
  const nextOnset = new Map<number, number>();
  for (const [index, onset] of onsets.entries()) {
    if (index + 1 < onsets.length) nextOnset.set(onset, onsets[index + 1]);
  }

  return mapNotes(pattern, lane, ids, (note) => {
    const next = nextOnset.get(note.startTick);
    return next === undefined
      ? note
      : { ...note, lenTicks: Math.max(1, next - note.startTick) };
  });
}

/**
 * Pull the selection towards the grid, by `strength`.
 *
 * `1` snaps hard, `0` changes nothing, and everything between moves each note
 * that fraction of the way — which is the control that lets a producer tighten
 * a performance without flattening the feel out of it. The same blend
 * `engine::humanize`'s `quantize_strength` applies to the generated pattern.
 */
export function quantize(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  gridTicks: number,
  strength: number,
): Pattern {
  if (gridTicks <= 0) return pattern;
  const amount = Math.min(1, Math.max(0, strength));
  if (amount === 0) return pattern;

  return mapNotes(pattern, lane, ids, (note) => {
    const target = Math.round(note.startTick / gridTicks) * gridTicks;
    return { ...note, startTick: note.startTick + (target - note.startTick) * amount };
  });
}

/**
 * Put human inconsistency back into a selection that has none.
 *
 * ⛔ **The randomness is an argument.** A transform that reached for
 * `Math.random` could not be asserted note by note, and the roadmap asks for
 * exactly that — so the caller passes the source and the tests pass a
 * deterministic one. `engine::humanize` makes the same choice for the same
 * reason.
 */
export function humanize(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  /** How far a note may move, in ticks, and how far its velocity may drift. */
  amount: { ticks: number; velocity: number },
  random: () => number = Math.random,
): Pattern {
  return mapNotes(pattern, lane, ids, (note) => {
    const drift = (range: number) => (random() * 2 - 1) * range;
    return {
      ...note,
      // Floored at zero here as well as in `clampNote`, so an early note at the
      // very start of the clip is pinned rather than wrapping.
      startTick: Math.max(0, note.startTick + drift(amount.ticks)),
      vel: Math.min(MAX_VELOCITY, Math.max(MIN_VELOCITY, note.vel + drift(amount.velocity))),
    };
  });
}

/**
 * Fold every out-of-key note to the nearest pitch that is in the key.
 *
 * ⛔ **Nearest, with a tie going up.** A note a semitone either side of the key
 * has two equally near answers, and picking arbitrarily makes the transform
 * unpredictable on exactly the notes a producer reaches for it to fix. Up is the
 * leading-tone reading and matches what Ableton does.
 */
export function transposeToScale(
  pattern: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
  /** The pitch classes in the key, as `scale.ts` publishes them. */
  classes: ReadonlySet<number>,
): Pattern {
  if (classes.size === 0) return pattern;

  return mapNotes(pattern, lane, ids, (note) => {
    if (classes.has(((note.pitch % 12) + 12) % 12)) return note;
    for (let distance = 1; distance <= 6; distance += 1) {
      for (const candidate of [note.pitch + distance, note.pitch - distance]) {
        if (candidate < 0 || candidate > 127) continue;
        if (classes.has(candidate % 12)) return { ...note, pitch: candidate };
      }
    }
    return note;
  });
}

/**
 * The ids a transform's own output should leave selected.
 *
 * ⛔ **A `NoteId` is a start tick and a pitch**, so every transform here except
 * `humanize`'s velocity drift renames the notes it touched. Without this the
 * selection would point at notes that no longer exist and the next transform in
 * a chain — invert, then legato — would silently do nothing.
 */
export function reselect(
  before: Pattern,
  after: Pattern,
  lane: Lane,
  ids: ReadonlySet<NoteId>,
): NoteId[] {
  const untouched = new Set(
    notesOf(before, lane)
      .filter((note) => !ids.has(noteId(note)))
      .map(noteId),
  );
  return notesOf(after, lane)
    .map(noteId)
    .filter((id) => !untouched.has(id));
}
