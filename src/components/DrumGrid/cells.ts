import type { Lane, Pattern } from '../../lib/ipc-types';
// ⛔ **The clip's length honours its meter, and this file used to assume common
// time** — `bars × ppq × 4`. Invisible until TASK-041E made the meter settable,
// and then wrong in two directions at once: the grid drew a third more columns
// than a 6/8 clip has, while the velocity lane below it (which does use this)
// placed its caps on the real tick scale. One definition, one answer.
import { patternTicks } from '../PianoRoll/notes';

/**
 * Turning a pattern into grid cells, kept out of the component file.
 *
 * Two reasons: fast refresh only works when a module exports components alone,
 * and this is the part with logic worth testing on its own — a note bucketed
 * one column late is a beat that looks right and plays wrong.
 */

/**
 * The order lanes are drawn in, top to bottom: the kit from the top down.
 *
 * ⚠ **Every lane the drum generator can write must appear here**, or a model
 * that authors it produces notes with no row to draw them in. That is the
 * frontend half of the guard `every_lane_the_drum_generator_writes_has_a_pad_to
 * _play_it` keeps on the audio side — this list and `drums.rs`'s `LANE_ORDER`
 * are the same set in a different order, deliberately: the engine builds
 * kick-first because the grammar hangs off it, and the grid draws
 * brightest-first because that is how a producer reads a kit.
 */
export const LANE_ORDER: Lane[] = [
  'closedHat',
  'pedalHat',
  'openHat',
  'ride',
  'rideBell',
  'crash',
  'triangle',
  'shaker',
  'tambourine',
  'snap',
  'clap',
  'snare',
  'offSnare',
  'ghostSnare',
  'rim',
  'woodblock',
  'clave',
  'cowbell',
  'timbale',
  'bongo',
  'conga',
  'perc',
  'perc2',
  'tomHigh',
  'tom',
  'tomLow',
  'kick',
  'subKick',
  'sub',
  'subLow',
  'riser',
  'impact',
  'reverse',
];

/**
 * The rows of this grid whose notes carry real pitch rather than a fixed drum
 * voice.
 *
 * ⛔ **Mirrors `engine::midi::is_pitched`, minus the four melodic parts** —
 * those are not rows of the drum grid at all, so they are absent from
 * `LANE_ORDER` and cannot reach anything here. `every_pitched_lane_gets_a_
 * channel_of_its_own` in `engine/src/midi.rs` is the list this must agree with;
 * `cells.test.ts` asserts these two are never offered as a drum slot.
 */
export const PITCHED_LANES: Lane[] = ['sub', 'subLow'];

/** Ticks per 16th note. `PPQ` is 960 in the engine, so a 16th is a quarter of it. */
export const TICKS_PER_16TH = 240;

/**
 * How early a note may sit and still belong to the cell it is decorating.
 *
 * The engine writes off the grid on purpose — humanize jitter, and the
 * `offGridMs` displacement some genres are *made* of — so a hit nudged early
 * must not fall back into the previous cell. 40 ticks is ~17 ms at 140 BPM,
 * comfortably past the largest jitter any model authors (jerk's 14 ms snare)
 * and nowhere near a 32nd, which is 120 ticks and has to stay its own hit.
 *
 * This is why the bucketing floors with a tolerance rather than rounding.
 * Rounding sends anything half a cell late into the *next* column, which turns
 * a 32nd hat roll — two hits inside one 16th — into a plain run of 16ths, and
 * the roll the generator worked to produce becomes invisible.
 */
const EARLY_TOLERANCE = 40;

export type Cell = { hits: number; velocity: number };
export type Row = { lane: Lane; cells: Cell[] };

/**
 * How busy each of `columns` slices of the pattern is, 0–1.
 *
 * This is what lets the generation ripple ignite cells *where the notes are*
 * rather than sweeping a uniform bar across the grid (FR-017). Normalised
 * against the busiest column, so a sparse boom-bap pattern lights up as much
 * as a dense drill one — the shape is what carries the information, not the
 * absolute count.
 */
export function columnDensity(pattern: Pattern, columns: number): number[] {
  const density = new Array<number>(Math.max(1, columns)).fill(0);
  const endTick = patternTicks(pattern);

  for (const track of pattern.lanes) {
    for (const note of track.notes) {
      const column = Math.floor((note.startTick / endTick) * density.length);
      if (column < 0 || column >= density.length) continue;
      density[column] += 1;
    }
  }

  const busiest = Math.max(...density);
  if (busiest <= 0) return density;
  return density.map((count) => count / busiest);
}

/**
 * Bucket a pattern's notes into 16th-note cells per lane.
 *
 * A cell carries how many hits landed in it as well as the loudest, because
 * the grid is 16ths and a roll is finer than that. Without the count, a 32nd
 * roll and one tap would draw identically.
 */
export function toCells(pattern: Pattern): Row[] {
  const columns = Math.max(1, Math.round(patternTicks(pattern) / TICKS_PER_16TH));

  return LANE_ORDER.filter((lane) => pattern.lanes.some((track) => track.lane === lane)).map(
    (lane) => {
      const cells: Cell[] = Array.from({ length: columns }, () => ({ hits: 0, velocity: 0 }));
      for (const track of pattern.lanes) {
        if (track.lane !== lane) continue;
        for (const note of track.notes) {
          const column = columnOf(note.startTick);
          if (column < 0 || column >= columns) continue;
          cells[column].hits += 1;
          cells[column].velocity = Math.max(cells[column].velocity, note.vel);
        }
      }
      return { lane, cells };
    },
  );
}

/**
 * Editing the grid (TASK-131G).
 *
 * Mike, 2026-08-05: *"we need a way to set rolls/delete rolls/set
 * hihats/kicks/snares where you want them/delete them, clone them, copy them,
 * etc., along with being able to create triplets, quintuplets, etc. with the
 * click of inserting a hi hat and pressing a button like 'Ctrl+3'"*.
 *
 * ⛔ **Pure functions on the `Pattern`, not on the cells.** A cell is a *view* —
 * it has already thrown away where inside the 16th a hit sat, which is exactly
 * what a triplet is made of. Editing the cells and rebuilding would quantise
 * every roll in the pattern to 16ths the first time anybody clicked anything.
 *
 * ⚠ **The 16th bucketing did NOT need replacing, and an earlier note claiming it
 * did was wrong.** `toCells` accumulates `hits` per cell, so three notes inside
 * one 16th draw as a three-hit cell — the same way the 32nd rolls the generator
 * already writes have always drawn. What a cell cannot show is *where* inside
 * itself the hits sit; that is a rendering limit, not a storage one, and it does
 * not stop a tuplet from being stored, played or exported correctly.
 */

/** A hit's velocity when the producer places one by hand. */
const PLACED_VELOCITY = 100;

/**
 * The pitch a hand-placed hit should carry.
 *
 * ⛔ **This was a flat `36` and that became a real defect the moment TASK-131D
 * landed.** Before it, a drum note's pitch was decorative — the sampler ignored
 * it for percussion. Now both `render_preview` and the offline stem renderer
 * transpose by `note.pitch - gm_drum_note(lane)`, so a hand-placed closed hat
 * carrying 36 against a lane rooted at 42 played **six semitones down**.
 *
 * ⚠ **Taken from the lane's own notes rather than from a table**, deliberately:
 * `engine::midi::gm_drum_note` is the one authority for what a lane sounds at,
 * and copying it here would be a second table for the page to keep in step —
 * the exact drift this codebase keeps writing down.
 *
 * ⛔ **`null` when the lane has no note to copy, and the caller must pass that
 * through rather than substituting a number.** An earlier version fell back to
 * `0`, which is not "no transposition" — the sampler transposes percussion by
 * `note.pitch - gm_drum_note(lane)`, so pitch 0 against a kick rooted at 36 or a
 * snap at 75 played 36 to 75 semitones down: sub-rumble, not a drum. A lane
 * emptied by hand keeps its (empty) track, so this is reachable in one click.
 * `plugin/src/audio/kit.rs`'s `semitones_for` treats a `0` on an unpitched lane
 * as "play it as sampled", which is what makes the `null` case safe.
 */
function placedPitch(pattern: Pattern, lane: Lane): number | null {
  return notesIn(pattern, lane)[0]?.pitch ?? null;
}

/**
 * Which column `toCells` will draw a note in.
 *
 * ⛔ **The one definition, because two of them was a real defect.** `toCells`
 * buckets with `EARLY_TOLERANCE` — the engine writes off the grid on purpose, so
 * a hit nudged early must not fall into the previous cell — and the edit
 * functions were written with an exact half-open 16th span instead. Roughly half
 * the hits in any *generated* pattern are jittered early, so the cell a producer
 * saw lit and the cell an edit targeted disagreed: clicking a lit cell appended a
 * second hit and played a flam, Delete on it was a silent no-op, and clicking the
 * visually empty cell to its left deleted the hit they could see.
 *
 * ⚠ The unit tests missed it because they used hand-built patterns with notes
 * exactly on the grid — the one shape the engine never produces.
 */
export function columnOf(startTick: number): number {
  return Math.floor((startTick + EARLY_TOLERANCE) / TICKS_PER_16TH);
}

/**
 * The ticks a cell spans, so an edit replaces exactly what is drawn in it.
 *
 * The inverse of [`columnOf`]: `[column·240 − 40, column·240 + 200)`.
 */
function cellSpan(column: number): [number, number] {
  const from = column * TICKS_PER_16TH - EARLY_TOLERANCE;
  return [from, from + TICKS_PER_16TH];
}

/** Replace one lane's notes, adding the lane if the pattern has no track yet. */
function withLane(pattern: Pattern, lane: Lane, notes: Pattern['lanes'][number]['notes']) {
  const existing = pattern.lanes.some((track) => track.lane === lane);
  const lanes = existing
    ? pattern.lanes.map((track) => (track.lane === lane ? { ...track, notes } : track))
    : [...pattern.lanes, { lane, notes }];
  // ⚠ A lane emptied by hand keeps its (empty) track rather than being dropped:
  // `toCells` only draws lanes the pattern *has*, so removing it would make the
  // row vanish under the producer's cursor mid-edit.
  return { ...pattern, lanes };
}

function notesIn(pattern: Pattern, lane: Lane) {
  return pattern.lanes.find((track) => track.lane === lane)?.notes ?? [];
}

/**
 * Turn a cell on if it is empty, off if it is not.
 *
 * ⚠ Off removes **every** hit in the cell, roll included. Removing one note of a
 * three-note roll and leaving two would be a state the producer cannot see —
 * the cell would still read as occupied — so the click means "clear this cell".
 */
export function toggleHit(pattern: Pattern, lane: Lane, column: number): Pattern {
  const [from, to] = cellSpan(column);
  const notes = notesIn(pattern, lane);
  const occupied = notes.some((note) => note.startTick >= from && note.startTick < to);

  if (occupied) {
    return withLane(
      pattern,
      lane,
      notes.filter((note) => !(note.startTick >= from && note.startTick < to)),
    );
  }
  return withLane(pattern, lane, [
    ...notes,
    {
      startTick: from + EARLY_TOLERANCE,
      lenTicks: TICKS_PER_16TH,
      pitch: placedPitch(pattern, lane) ?? 0,
      vel: PLACED_VELOCITY,
      modelVel: null,
      slideToPitch: null,
      articulation: null,
    },
  ]);
}

/**
 * Split a cell into `count` evenly spaced hits — `Ctrl+3` a triplet, `Ctrl+5` a
 * quintuplet.
 *
 * ⛔ **The one control that genuinely needs sub-16th storage**, and the reason
 * the edits operate on ticks. A triplet inside a 16th is three notes at +0, +80
 * and +160 ticks; nothing about that is expressible as "a cell".
 *
 * ⚠ The pitch and velocity come from whatever was already in the cell, so
 * turning a hi-hat into a triplet gives three hi-hats rather than three of the
 * default. An empty cell gets the default, which is what makes `Ctrl+3` work as
 * "put a triplet here" as well as "make this one a triplet".
 */
export function tuplet(pattern: Pattern, lane: Lane, column: number, count: number): Pattern {
  const size = Math.max(2, Math.min(16, Math.round(count)));
  const [from, to] = cellSpan(column);
  const notes = notesIn(pattern, lane);
  const inside = notes.filter((note) => note.startTick >= from && note.startTick < to);
  const template = inside[0];
  // ⚠ **The true subdivision, not one squeezed to fit the drawing.** A 16th
  // triplet is 80 ticks; narrowing it so every note falls in one column would
  // make it not a triplet. A sextuplet or finer does put its last note in the
  // next column — and that is fine now, because  is the inverse of
  // , so clicking the cell it is drawn in finds and clears it. Before
  // that, the stray hit was unreachable from where the producer could see it.
  const step = TICKS_PER_16TH / size;

  const replacement = Array.from({ length: size }, (_, index) => ({
    startTick: Math.round(from + EARLY_TOLERANCE + index * step),
    // ⚠ Rounded up so consecutive tuplet notes cannot overlap into a
    // zero-length gap, which `pattern_to_smf` pairs note-offs against.
    lenTicks: Math.max(1, Math.floor(step)),
    pitch: template?.pitch ?? placedPitch(pattern, lane) ?? 0,
    vel: template?.vel ?? PLACED_VELOCITY,
    modelVel: null,
    slideToPitch: null,
    articulation: null,
  }));

  return withLane(pattern, lane, [
    ...notes.filter((note) => !(note.startTick >= from && note.startTick < to)),
    ...replacement,
  ]);
}

/**
 * Clear one cell, if it has anything in it.
 *
 * ⚠ Its own export rather than a caller re-deriving the span: the Delete
 * shortcut needs to know whether a cell is occupied *before* acting, and doing
 * that in the component meant copying ,  and the occupancy
 * predicate out of this file — with  written as a literal — which
 * is exactly the tick arithmetic this module exists to keep in one place.
 */
export function clearCell(pattern: Pattern, lane: Lane, column: number): Pattern {
  const [from, to] = cellSpan(column);
  const notes = notesIn(pattern, lane);
  if (!notes.some((note) => note.startTick >= from && note.startTick < to)) return pattern;
  return withLane(
    pattern,
    lane,
    notes.filter((note) => !(note.startTick >= from && note.startTick < to)),
  );
}

/**
 * Copy one bar of one lane over another bar of the same lane.
 *
 * ⚠ **The destination is cleared first.** Merging would double every hit the two
 * bars share, which reads as the clone having worked and sounds like a flam.
 */
export function cloneBar(
  pattern: Pattern,
  lane: Lane,
  fromBar: number,
  toBar: number,
): Pattern {
  const barTicks = Math.max(1, Math.round(patternTicks(pattern) / Math.max(1, pattern.bars)));
  const sourceFrom = fromBar * barTicks;
  const destFrom = toBar * barTicks;
  const notes = notesIn(pattern, lane);

  // ⛔ **Tolerance-aware on both ends, for the reason `columnOf` gives.** A
  // downbeat that `humanize` nudged early sits just *below* the bar line, so an
  // exact boundary left it in place while the copy landed on the line — two hits
  // a few ticks apart, which is an audible flam from the one gesture whose whole
  // contract is that the destination is cleared first.
  const inBar = (tick: number, start: number) =>
    tick >= start - EARLY_TOLERANCE && tick < start + barTicks - EARLY_TOLERANCE;

  const copied = notes
    .filter((note) => inBar(note.startTick, sourceFrom))
    .map((note) => ({ ...note, startTick: note.startTick - sourceFrom + destFrom }));

  return withLane(pattern, lane, [
    ...notes.filter((note) => !inBar(note.startTick, destFrom)),
    ...copied,
  ]);
}

/**
 * How many 16ths one fill covers: the last beat of the phrase.
 *
 * ⛔ **A beat, and the same beat the engine's fill uses.** `rolls::hat_fills`
 * puts its window at `bar_ticks - ticks_per_beat`, so an "add fill" that wrote
 * anywhere else would produce a figure the generator would never have written —
 * and a producer who then regenerated would watch theirs move.
 *
 * ⛔⛔ **Which is why it cannot be the constant 4 it used to be.** A beat is
 * `PPQ * 4 / timeSigDen`, so it is four 16ths in x/4 and *two* in x/8. In 6/8
 * the hardcoded four cleared and rewrote two beats of hats — twice the figure
 * the engine would ever write, over notes the producer never asked to lose —
 * and the promise made three lines up was false for every meter but one.
 */
function fillSixteenths(pattern: Pattern): number {
  const den = pattern.timeSigDen === 0 ? 4 : pattern.timeSigDen;
  // The same fallback `engine::context::normalise_meter` states: 1 is a *legal*
  // denominator, so clamping to 1 would accept a malformed clip and make the
  // beat four times too long rather than rejecting it.
  return Math.max(1, Math.round((pattern.ppq * 4) / den / TICKS_PER_16TH));
}

/**
 * How many hits per 16th an added fill writes — a 32nd stream.
 *
 * ⚠ Here rather than passed in. It lived in `DrumGrid.tsx` and travelled as an
 * argument, which made `addFill` look configurable when there was exactly one
 * caller and one value; the fill's shape is one idea and belongs beside the
 * window it is written into.
 */
const FILL_HITS = 2;

/**
 * Write a fill into the last beat of the pattern (TASK-043H).
 *
 * The per-lane half of the fill palette: the hat is where trap, drill and plugg
 * do their talking, and this is the phrase-end figure that breaks the stream
 * and hands over to the next bar.
 *
 * `FILL_HITS` is a count *per 16th*, so it is the same unit the roll palette
 * uses — one vocabulary across both, rather than a second reading of the
 * engine's roll names on the page.
 *
 * ⛔ **Velocity ramps across the whole figure, not per cell.** A fill is one
 * gesture: four separate cells each ramping 45→100 would read as four little
 * crescendos rather than one hand-over, which is the difference between a fill
 * and a busy bar.
 */
export function addFill(pattern: Pattern, lane: Lane): Pattern {
  const sixteenths = fillSixteenths(pattern);
  const columns = Math.max(1, Math.round(patternTicks(pattern) / TICKS_PER_16TH));
  const first = Math.max(0, columns - sixteenths);
  const step = TICKS_PER_16TH / FILL_HITS;

  const notes = notesIn(pattern, lane);
  // ⛔⛔ **Two boundaries, and collapsing them into one wrote every fill 40
  // ticks early.** `cellSpan` deliberately reaches back by `EARLY_TOLERANCE` so
  // a *humanized* hit that landed just ahead of its 16th still reads as being
  // in that cell — right for deciding what the window already contains, wrong
  // for deciding where to put a note. Using it for both meant the figure began
  // before the beat, and the promise two comments up — that a hand-added fill
  // lands exactly where `rolls::hat_fills` puts a generated one — was false for
  // every fill this function has ever written.
  const [clearFrom] = cellSpan(first);
  const from = first * TICKS_PER_16TH;
  const to = patternTicks(pattern);
  // Whatever the lane already plays, so a fill on the hats is hats.
  const template = notes.find((note) => note.startTick < clearFrom);
  const pitch = template?.pitch ?? placedPitch(pattern, lane) ?? 0;

  const total = sixteenths * FILL_HITS;
  const written = Array.from({ length: total }, (_, index) => ({
    startTick: Math.round(from + index * step),
    lenTicks: Math.max(1, Math.floor(step)),
    pitch,
    // 45% to full across the figure — the same shape `hihat.fill`'s default
    // `rampRange` writes, so an added fill and a generated one sound alike.
    vel: Math.round(57 + (127 - 57) * (index / Math.max(1, total - 1))),
    modelVel: null,
    slideToPitch: null,
    articulation: null,
  })).filter((note) => note.startTick < to);

  return withLane(pattern, lane, [
    ...notes.filter((note) => note.startTick < clearFrom),
    ...written,
  ]);
}

/**
 * The lanes this pattern has no row for (TASK-043A).
 *
 * ⛔ **The unused ones only, and that is the rule the picker is built on.** Two
 * slots claiming the same lane is a pattern where one of them silently never
 * sounds: the audio thread finds the first pad for a lane and the second row's
 * notes go to the same voice, so the producer edits a row they cannot hear.
 * Offering only the free lanes makes that state unreachable rather than
 * something to validate afterwards.
 *
 * ⚠ **The four melodic lanes cannot appear, and it is [`LANE_ORDER`] that
 * guarantees it rather than a filter here.** They are *parts*, not kit slots — a
 * drum row reassigned to `melody` would put unpitched hits on the lead's pad.
 * The first cut filtered them out explicitly, which read as a safeguard and was
 * dead code: `LANE_ORDER` holds no melodic lane, so the filter could never
 * remove one and the test for it passed vacuously.
 * `orders every lane the engine can produce` in `cells.test.ts` is what keeps
 * that true — a melodic lane added to `LANE_ORDER` fails there.
 *
 * ⛔⛔ **…and the 808s got through on exactly that reasoning.** `sub` and
 * `subLow` *are* rows of this grid, so `LANE_ORDER` rightly holds them — but
 * they are **pitched**, and the exclusion above is about pitch rather than
 * about being a part. `reassignLane` moves notes unchanged, on purpose, so a
 * perc row picked over to "808" exported its drum hits down the pitched channel
 * as bass notes at whatever pitch they happened to carry. The rule the comment
 * always described is now the rule the code applies.
 */
export function unusedLanes(pattern: Pattern): Lane[] {
  const taken = new Set(pattern.lanes.map((track) => track.lane));
  return LANE_ORDER.filter((lane) => !taken.has(lane) && !PITCHED_LANES.includes(lane));
}

/**
 * Move a slot's notes to a different lane (TASK-043A).
 *
 * ⛔ **The row keeps its position in `LANE_ORDER`, because the order is the
 * kit's rather than the pattern's.** Reassigning the rim to a cowbell moves the
 * row to where a cowbell belongs, which is the honest answer: the grid draws a
 * kit top-down, and a cowbell drawn among the snares would be a row in the
 * wrong place for the rest of the session.
 *
 * Refuses when the target is already in use — see [`unusedLanes`] — rather than
 * merging two slots into one, which would silently destroy one of them.
 */
export function reassignLane(pattern: Pattern, from: Lane, to: Lane): Pattern {
  if (from === to) return pattern;
  if (pattern.lanes.some((track) => track.lane === to)) return pattern;
  const moving = pattern.lanes.find((track) => track.lane === from);
  if (moving === undefined) return pattern;

  return {
    ...pattern,
    // ⚠ The *notes* move unchanged: a hit is a hit, and re-pitching them to the
    // new lane's GM note is the exporter's job (`gm_drum_note`), not this one.
    // Doing it here would bake a drum map into the clip and break the moment a
    // producer put their own sample on the pad.
    lanes: pattern.lanes.map((track) => (track.lane === from ? { ...track, lane: to } : track)),
  };
}
