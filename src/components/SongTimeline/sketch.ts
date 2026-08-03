/**
 * The note-density sketch drawn inside a clip (TASK-070).
 *
 * ⛔ **A painted gradient, not a canvas and not one element per bucket — the
 * same decision the grid in this view already makes, for the same reason.** A
 * structure may hold 64 sections over 5 part rows, so a clip is up to 320
 * places on screen; a canvas each means 320 contexts created and drawn
 * synchronously in one render, and an element per bucket means thousands of
 * nodes. Both are the "takes the DAW down" shape this project has been bitten
 * by. A gradient is one string per clip and no extra nodes at any song length.
 *
 * The sketch answers one question — *where in this clip is anything
 * happening?* — so it is deliberately coarse. A producer reading it is
 * distinguishing a busy hook from a sparse intro at a glance, not counting
 * notes.
 */

import type { Pattern } from '../../lib/ipc-types';

/**
 * How many columns the clip is divided into.
 *
 * Sixteen, because a bar of 16ths is the resolution a beat is thought about in
 * — the same reasoning `DrumGrid`'s cells document. Finer buckets do not
 * survive being drawn a few pixels wide, and coarser ones stop distinguishing a
 * four-on-the-floor from a half-time beat.
 */
export const BUCKETS = 16;

/**
 * Note density per bucket, each `0`–`1` against the busiest bucket.
 *
 * Relative rather than absolute: a clip is being compared against *itself* —
 * where its own activity sits — and a chord pad holding four long notes would
 * otherwise sketch as almost nothing beside a hi-hat lane.
 */
export function density(pattern: Pattern): number[] {
  const span = patternSpan(pattern);
  const buckets = new Array<number>(BUCKETS).fill(0);
  if (span <= 0) return buckets;

  for (const lane of pattern.lanes) {
    for (const note of lane.notes) {
      // Clamped rather than skipped: a note exactly at the end of the clip is
      // real, and dropping it would make the last column read as silence.
      const at = Math.min(BUCKETS - 1, Math.floor((note.startTick / span) * BUCKETS));
      if (at >= 0) buckets[at] += 1;
    }
  }

  const peak = Math.max(...buckets);
  return peak > 0 ? buckets.map((count) => count / peak) : buckets;
}

/**
 * The clip's own length in ticks.
 *
 * ⚠ Derived from the meter rather than assumed to be four quarters to the bar.
 * `patternTicks` in the roll had exactly this bug and disagreed with the engine
 * for every meter but 4/4 — a 6/8 clip sketched with its notes bunched into the
 * first two thirds and the rest blank.
 */
function patternSpan(pattern: Pattern): number {
  const den = pattern.timeSigDen === 0 ? 4 : pattern.timeSigDen;
  const perBar = Math.max(1, (pattern.ppq * 4) / den) * Math.max(1, pattern.timeSigNum);
  return perBar * Math.max(1, pattern.bars);
}

/**
 * The gradient that paints `density` across a clip's width.
 *
 * Hard stops rather than a smooth ramp: the buckets are discrete and blending
 * them would suggest a resolution the sketch does not have.
 */
export function sketchGradient(levels: number[]): string {
  if (levels.length === 0) return 'none';
  const step = 100 / levels.length;
  const stops = levels.map((level, index) => {
    // A floor of zero stays fully transparent, so an empty bar reads as empty
    // rather than as "quiet".
    const alpha = level === 0 ? 0 : 0.18 + level * 0.5;
    const from = (index * step).toFixed(4);
    const to = ((index + 1) * step).toFixed(4);
    return `rgba(255, 255, 255, ${alpha.toFixed(3)}) ${from}% ${to}%`;
  });
  return `linear-gradient(to right, ${stops.join(', ')})`;
}
