/**
 * The FX cascade's levels (TASK-073).
 *
 * ⛔ **This module used to also draw the clips**, as a note-*density* gradient —
 * sixteen buckets of "how busy is this bar" painted behind the label. TASK-142
 * replaced that with the notes themselves (`clipArt.ts`), because Mike's review
 * found the honest version of what it was: *"a clip does not look like a clip"*.
 * `density` and `sketchGradient` went with it rather than being left as dead
 * code; `sectionDensity` is a different thing and still has a caller.
 *
 * ⚠ **The node-count reasoning that produced the gradient is still true and now
 * lives in `clipArt.ts`.** A structure may hold 64 sections over 5 rows, so a
 * clip is up to 320 places on screen — which is why the notes are one SVG path
 * per repeat rather than an element per note or a canvas per clip.
 */

import type { Song } from '../../lib/ipc-types';

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
