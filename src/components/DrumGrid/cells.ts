import type { Lane, Pattern } from '../../lib/ipc-types';

/**
 * Turning a pattern into grid cells, kept out of the component file.
 *
 * Two reasons: fast refresh only works when a module exports components alone,
 * and this is the part with logic worth testing on its own — a note bucketed
 * one column late is a beat that looks right and plays wrong.
 */

/** The order lanes are drawn in, top to bottom: the kit from the top down. */
export const LANE_ORDER: Lane[] = [
  'closedHat',
  'openHat',
  'snap',
  'clap',
  'snare',
  'rim',
  'perc',
  'kick',
  'bass808',
];

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
  const endTick = Math.max(1, pattern.bars * pattern.ppq * 4);

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
  const columns = Math.max(1, Math.round((pattern.bars * pattern.ppq * 4) / TICKS_PER_16TH));

  return LANE_ORDER.filter((lane) => pattern.lanes.some((track) => track.lane === lane)).map(
    (lane) => {
      const cells: Cell[] = Array.from({ length: columns }, () => ({ hits: 0, velocity: 0 }));
      for (const track of pattern.lanes) {
        if (track.lane !== lane) continue;
        for (const note of track.notes) {
          const column = Math.floor((note.startTick + EARLY_TOLERANCE) / TICKS_PER_16TH);
          if (column < 0 || column >= columns) continue;
          cells[column].hits += 1;
          cells[column].velocity = Math.max(cells[column].velocity, note.vel);
        }
      }
      return { lane, cells };
    },
  );
}
