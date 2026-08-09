/**
 * What a clip looks like: its actual notes, and whether it is MIDI or audio
 * (TASK-142).
 *
 * ⛔⛔ **Mike's finding, verbatim: *"a clip does not look like a clip"*, and
 * *"being able to see the actual midi clip chips and the audio clip chips, not
 * just words on the timeline"*.** What the timeline drew was a label over a
 * note-*density* gradient — sixteen buckets of "how busy is this bar", which
 * says nothing about pitch, nothing about where a hit lands and nothing about
 * which of two clips you are looking at. Every DAW draws the notes, because that
 * is the thing a producer recognises a clip by.
 *
 * ⛔ **One SVG path per clip, and the node-count argument that produced the
 * gradient still holds.** A structure may hold 64 sections over 5 rows, so a
 * clip is up to 320 places on screen; an element per note is tens of thousands
 * of nodes and a canvas each is 320 contexts drawn synchronously in one render
 * — both the "takes the DAW down" shape this project has been bitten by. A path
 * is *one* node however many notes it holds, and unlike the gradient it can say
 * where they are.
 */

import type { Lane, Pattern } from '../../lib/ipc-types';
import { patternTicks } from '../PianoRoll/notes';

/** The box the notes are drawn into. Unitless — the SVG scales to the clip. */
export const CLIP_W = 100;
export const CLIP_H = 100;

/**
 * The pitch range a melodic clip is drawn across when it has no spread of its
 * own.
 *
 * ⚠ **A floor, not a window.** A bassline that sits on two notes would otherwise
 * draw one blob at the top and one at the bottom, which reads as a huge leap;
 * spreading it over at least an octave keeps a narrow line looking narrow.
 */
const MIN_PITCH_SPAN = 12;

/** How tall one note is drawn, as a fraction of the box. */
const NOTE_H = 6;

/**
 * The notes of `pattern` as one SVG path, in a `0 0 100 100` box.
 *
 * Time runs left to right across the clip's own length; pitch runs bottom to
 * top. A drum clip has little pitch spread by nature, so it comes out as rows of
 * hits — which is exactly how a drum clip looks in Ableton.
 */
/**
 * One path per *pattern*, not per clip.
 *
 * ⛔ **Sections of the same kind share one `Pattern`** — that is `arrange.rs`'s
 * second stated decision and the whole reason a `PatternRef` is an id. So a
 * 64-section song draws ~320 clips out of ~25 distinct patterns, and memoising
 * inside the component (which is where this started) ran the walk 320 times to
 * produce 25 strings: ~35k note iterations and ~1 MB of string garbage on every
 * generate, undo and project load, for ~2.7k iterations' worth of answer.
 *
 * ⚠ **A `WeakMap`, so a pattern that falls out of the song takes its path with
 * it.** Keyed on exactly the object identity the component's `useMemo` already
 * keys on, so nothing about staleness changes: a re-roll or an edit replaces the
 * object and gets a fresh path.
 */
const PATHS = new WeakMap<Pattern, Map<number, string>>();

/**
 * The clip's notes, drawn across the `loopBars` bars it actually repeats on.
 *
 * ⛔⛔ **`loopBars`, not the pattern's own length, and getting that wrong drew
 * notes the engine refuses to play.** TASK-142 lets a clip loop on fewer bars
 * than its pattern holds, and the timeline tiles this path once per repeat — so
 * scaling to the full pattern squeezed all four bars of a four-bar clip into
 * each one-bar tile. What was on screen then disagreed with both the transport
 * and the export, which is the readout-that-lies shape this view guards against
 * everywhere else. `SectionTiling::sounds` drops anything whose onset is at or
 * past the loop point; this drops exactly the same notes.
 *
 * ⚠ Cached per `(pattern, loopBars)` — a resize is a new window over the same
 * object, so keying the memo on the pattern alone would hand back the old
 * picture.
 */
export function notesPath(pattern: Pattern | null, loopBars: number): string {
  if (!pattern) return '';
  const bars = loopBars > 0 ? Math.min(loopBars, pattern.bars) : pattern.bars;
  let windows = PATHS.get(pattern);
  if (!windows) {
    windows = new Map();
    PATHS.set(pattern, windows);
  }
  const cached = windows.get(bars);
  if (cached !== undefined) return cached;
  const built = buildPath(pattern, bars);
  windows.set(bars, built);
  return built;
}

function buildPath(pattern: Pattern, bars: number): string {
  const notes = pattern.lanes.flatMap((track) => track.notes);
  if (notes.length === 0) return '';

  // ⛔ **`patternTicks`, never a private copy of the arithmetic.** This app has
  // exactly one definition of how long a clip is, and the module this file
  // replaced said why: *"a private copy here would have been the ninth"*.
  // `previewLayout.ts` re-exports rather than copies for the same reason. The
  // meter bug — computing a bar as four quarters — has shipped once already, and
  // `timeSigDen` handling lives only in `barTicks`.
  // ⚠ Scaled to the *loop*: `patternTicks / pattern.bars` is one bar, whatever
  // the meter, and the window is however many of those the clip repeats on.
  const ticks = Math.max(1, (patternTicks(pattern) / Math.max(1, pattern.bars)) * bars);

  let low = Infinity;
  let high = -Infinity;
  for (const note of notes) {
    if (note.pitch < low) low = note.pitch;
    if (note.pitch > high) high = note.pitch;
  }
  const middle = (low + high) / 2;
  const span = Math.max(MIN_PITCH_SPAN, high - low);
  const bottom = middle - span / 2;

  const parts: string[] = [];
  for (const note of notes) {
    const x = (note.startTick / ticks) * CLIP_W;
    if (x >= CLIP_W) continue;
    // ⚠ **A minimum width, because most of these are one 16th at 6 px/bar.**
    // Scaled honestly a hi-hat would be a hundredth of a pixel and the clip
    // would draw empty — which is the failure mode the density sketch was
    // reaching for and solved by not drawing notes at all.
    const w = Math.max(1.2, Math.min(CLIP_W - x, (note.lenTicks / ticks) * CLIP_W));
    const fromBottom = (note.pitch - bottom) / span;
    const y = CLIP_H - NOTE_H - Math.max(0, Math.min(1, fromBottom)) * (CLIP_H - NOTE_H);
    parts.push(`M${x.toFixed(2)} ${y.toFixed(2)}h${w.toFixed(2)}v${NOTE_H}h-${w.toFixed(2)}z`);
  }
  return parts.join('');
}

/**
 * What a clip can be handed to a DAW as.
 *
 * ⛔⛔ **Mike's second finding: *"MIDI and audio are indistinguishable — an
 * arrangement clip has no format at all"*.** Every clip in an arrangement is a
 * `Pattern`, so the honest distinction is not "is this file MIDI or a wav" — it
 * is **what this clip can be spooled as**, which is the question the Stems
 * panel's two handles have always asked and the timeline never did.
 *
 * ⚠ **The two answers come from different questions**, and `DragRows` records
 * why at length: MIDI asks *what was written* — notes are notes whether or not
 * this build can play them — and audio asks *what can be rendered*, which needs
 * a sample behind the lane. A melodic part has no voice in the shipped kit, so
 * it is MIDI-only and the clip says so rather than offering a drag that spools
 * silence.
 */
export type ClipFormat = {
  /** Always true for an arrangement clip: it is notes. */
  midi: boolean;
  /** Whether anything in it can be rendered through the kit. */
  audio: boolean;
};

/**
 * Which formats this clip offers, given the lanes the kit can actually sound.
 *
 * ⚠ **An unread kit answers "yes"**, the same way `DragRows.audible` does and
 * for the same reason: the cost of being wrong that way is one badge shown for a
 * fraction of a second, and the cost of the other way is the audio handle
 * vanishing from the panel with nothing on screen saying why.
 */
export function clipFormat(
  pattern: Pattern | null,
  soundable: ReadonlySet<Lane>,
  kitLoaded: boolean,
): ClipFormat {
  if (!pattern) return { midi: false, audio: false };
  const written = pattern.lanes.filter((track) => track.notes.length > 0);
  return {
    midi: written.length > 0,
    audio:
      written.length > 0 && (!kitLoaded || written.some((track) => soundable.has(track.lane))),
  };
}
