/**
 * The note-density sketch drawn inside a clip, and the FX cascade's levels
 * (TASK-070 / TASK-073).
 *
 * ⛔ **Painted as a gradient, not a canvas and not one element per bucket — the
 * same decision the grid in this view already makes, for the same reason.** A
 * structure may hold 64 sections over 5 part rows, so a clip is up to 320
 * places on screen; a canvas each means 320 contexts created and drawn
 * synchronously in one render, and an element per bucket means thousands of
 * nodes. Both are the "takes the DAW down" shape this project has been bitten
 * by. A gradient is one string per clip and no extra nodes at any song length.
 *
 * ⛔ **The counting is `columnDensity`'s, not a second copy.** That function
 * already buckets a pattern's notes and normalises them against the busiest
 * bucket — including the "relative to the clip itself, so a chord pad does not
 * lose to a hat lane" rule, which its own comment states. It also derives the
 * clip's length through `patternTicks`, which is the one definition of that in
 * the app; a private copy here would have been the ninth, and would have gone
 * stale the next time a meter question came up.
 */

import type { Pattern, Song } from '../../lib/ipc-types';
import { columnDensity } from '../DrumGrid/cells';

/**
 * How many columns the clip is divided into.
 *
 * Sixteen, because a bar of 16ths is the resolution a beat is thought about in
 * — the same reasoning `DrumGrid`'s cells document. Finer buckets do not
 * survive being drawn a few pixels wide, and coarser ones stop distinguishing a
 * four-on-the-floor from a half-time beat.
 */
export const BUCKETS = 16;

/** Note density per bucket, each `0`–`1` against the clip's busiest bucket. */
export function density(pattern: Pattern): number[] {
  return columnDensity(pattern, BUCKETS);
}

/**
 * One level per section, for the generation FX to cascade over (TASK-073).
 *
 * ⛔ **The same array shape the ripple already takes, so Song Mode reuses the
 * animation rather than growing a second one.** `GenFx` sweeps left to right and
 * lights each column by its own level; a song only has to say what a column
 * means here. A second animation would have needed a second reduced-motion
 * path, which is the half this project has already had to fix twice.
 *
 * Weighted by how many parts a section plays rather than by its notes: the
 * cascade runs *while the song is being built*, so the notes do not exist yet —
 * and what a producer is watching for is the shape filling in.
 */
export function sectionDensity(song: Song): number[] {
  const counts = song.sections.map((section) => Object.keys(section.patterns).length);
  const peak = Math.max(1, ...counts);
  // A floor, so a one-part intro still lights rather than reading as a gap in
  // the sweep — the same reason `GenFx` gives an empty column a base glow.
  return counts.map((count) => Math.max(0.25, count / peak));
}

/**
 * The gradient that paints `levels` across a clip's width.
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
