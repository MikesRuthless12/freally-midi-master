/**
 * The velocity lane's arithmetic (TASK-041V).
 *
 * Pure and separate from the canvas for the same reason `geometry.ts` is: the
 * renderer draws the caps and the pointer handlers hit-test them, and the two
 * agreeing is the difference between a lane you can edit and one where the
 * handle is a few pixels from where it looks. Everything here is in *lane*
 * coordinates — x from whatever tick mapping the editor above uses, y from the
 * lane's own height — so the same functions serve the roll and the drum grid.
 *
 * ⛔ **A stem knows which lane its note belongs to.** Under the roll that is one
 * lane and looks like ceremony; under the drum grid the lane shows the kick and
 * the 808 side by side, and a value written back to the wrong lane would move a
 * note the producer was not touching.
 */

import type { Lane, Note } from '../../lib/ipc-types';
import { MAX_VELOCITY, MIN_VELOCITY, noteId } from './notes';

/** The lane's height in CSS pixels — set on the element, so there is one copy. */
export const LANE_HEIGHT = 96;

/** The round handle at the top of each stem. */
export const CAP_RADIUS = 4;
/** The stem itself: a value, not a control. The roadmap asks for 1–2 px. */
export const STEM_WIDTH = 2;

/**
 * Room kept above and below the value range.
 *
 * Without the top pad a velocity of 127 draws its cap half off the canvas, and
 * "as loud as it goes" then looks like a note that is somehow louder still.
 */
const PAD_TOP = CAP_RADIUS + 2;
const PAD_BOTTOM = 2;

/** How far from a cap's centre a pointer still grabs it. */
export const GRAB_PX = CAP_RADIUS + 4;

/** Where a velocity's cap centre sits, measured from the top of the lane. */
export function velocityToY(vel: number, height: number): number {
  const floor = height - PAD_BOTTOM;
  const span = Math.max(1, floor - PAD_TOP);
  const t = (clampVelocity(vel) - MIN_VELOCITY) / (MAX_VELOCITY - MIN_VELOCITY);
  return floor - t * span;
}

/** What velocity a pointer at `y` is asking for. */
export function yToVelocity(y: number, height: number): number {
  const floor = height - PAD_BOTTOM;
  const span = Math.max(1, floor - PAD_TOP);
  const t = (floor - y) / span;
  return clampVelocity(MIN_VELOCITY + t * (MAX_VELOCITY - MIN_VELOCITY));
}

export function clampVelocity(vel: number): number {
  return Math.min(MAX_VELOCITY, Math.max(MIN_VELOCITY, Math.round(vel)));
}

/** One lane's notes, as the velocity lane is given them. */
export type Track = { lane: Lane; notes: readonly Note[] };

/** One note's slider: which note it belongs to and where its cap is drawn. */
export type Stem = {
  /** Unique across lanes, which `noteId` alone is not. */
  id: string;
  lane: Lane;
  note: Note;
  /** Cap centre, in lane pixels. */
  x: number;
};

/** A stem's key. Lane-qualified so two lanes cannot claim the same slider. */
export function stemId(lane: Lane, note: Note): string {
  return `${lane}:${noteId(note)}`;
}

/**
 * Lay out one slider per note.
 *
 * ⛔ **Notes on the same tick are spread sideways rather than stacked in
 * place.** A kick and an 808 on beat 1 are two values, and drawing them at the
 * same x means the louder cap hides the quieter one entirely — the lane then
 * silently stops being editable for the hidden note, which is worse than not
 * drawing it. Ordered by lane and then pitch so a note keeps its slot between
 * renders instead of swapping places with its neighbour as either value moves.
 */
export function stemsFor(tracks: readonly Track[], xOf: (tick: number) => number): Stem[] {
  const byTick = new Map<number, { lane: Lane; note: Note }[]>();
  for (const track of tracks) {
    for (const note of track.notes) {
      const at = byTick.get(note.startTick);
      if (at === undefined) byTick.set(note.startTick, [{ lane: track.lane, note }]);
      else at.push({ lane: track.lane, note });
    }
  }

  const stems: Stem[] = [];
  for (const [tick, group] of byTick) {
    const ordered = [...group].sort(
      (a, b) => a.lane.localeCompare(b.lane) || a.note.pitch - b.note.pitch,
    );
    const x = xOf(tick);
    for (const [index, { lane, note }] of ordered.entries()) {
      stems.push({ id: stemId(lane, note), lane, note, x: x + index * (2 * CAP_RADIUS + 1) });
    }
  }
  return stems;
}

/** The cap nearest `x`, if the pointer is close enough to have meant it. */
export function stemNear(stems: readonly Stem[], x: number, within = GRAB_PX): Stem | null {
  let best: Stem | null = null;
  let distance = within;
  for (const stem of stems) {
    const gap = Math.abs(stem.x - x);
    if (gap <= distance) {
      best = stem;
      distance = gap;
    }
  }
  return best;
}

/** A velocity per stem — what a gesture is asking the pattern to become. */
export type Values = Record<string, number>;

/**
 * The stems a sideways drag just passed over, all taking the pointer's height.
 *
 * ⛔ **Only the span between the last pointer event and this one**, not the
 * whole gesture. Repainting everything since the drag started at the *current*
 * height would flatten a slope the moment the pointer changed row — which is
 * the opposite of the gesture's purpose, since dragging on a slope is how a
 * producer writes a crescendo in one movement.
 */
export function sweep(stems: readonly Stem[], fromX: number, toX: number, vel: number): Values {
  const lo = Math.min(fromX, toX);
  const hi = Math.max(fromX, toX);
  const values: Values = {};
  for (const stem of stems) {
    if (stem.x >= lo - GRAB_PX && stem.x <= hi + GRAB_PX) values[stem.id] = clampVelocity(vel);
  }
  return values;
}

/**
 * `Shift`: a straight line from where the drag started to where it is now.
 *
 * Recomputed across the whole span every time rather than accumulated, because
 * the line is defined by its two ends — dragging back up has to redraw the
 * ramp, not leave the first version of it underneath.
 */
export function line(
  stems: readonly Stem[],
  from: { x: number; vel: number },
  to: { x: number; vel: number },
): Values {
  const lo = Math.min(from.x, to.x);
  const hi = Math.max(from.x, to.x);
  const run = to.x - from.x;
  const values: Values = {};
  for (const stem of stems) {
    if (stem.x < lo - GRAB_PX || stem.x > hi + GRAB_PX) continue;
    const t = run === 0 ? 1 : (stem.x - from.x) / run;
    values[stem.id] = clampVelocity(
      from.vel + (to.vel - from.vel) * Math.min(1, Math.max(0, t)),
    );
  }
  return values;
}

/**
 * Move a whole selection by one delta, keeping its shape.
 *
 * ⛔ **The delta is clamped once for the group, exactly as `moveNotes` clamps a
 * transpose.** Clamping each note on its own lets the quiet ones keep falling
 * after the loud ones have hit 127, which levels the accent pattern the
 * producer is dragging *because* they want to keep it.
 */
export function shift(stems: readonly Stem[], ids: ReadonlySet<string>, delta: number): Values {
  const picked = stems.filter((stem) => ids.has(stem.id));
  if (picked.length === 0) return {};

  const highest = Math.max(...picked.map((stem) => stem.note.vel));
  const lowest = Math.min(...picked.map((stem) => stem.note.vel));
  const room = Math.min(
    Math.max(delta, MIN_VELOCITY - lowest),
    Math.max(0, MAX_VELOCITY - highest),
  );

  const values: Values = {};
  for (const stem of picked) values[stem.id] = clampVelocity(stem.note.vel + room);
  return values;
}

/**
 * Put notes back to the velocity their model wrote.
 *
 * A note with no `modelVel` is one the producer drew themselves, and there is
 * no model opinion to return to — so it is left exactly as it is rather than
 * reset to some invented constant.
 */
export function reset(stems: readonly Stem[], ids: ReadonlySet<string>): Values {
  const values: Values = {};
  for (const stem of stems) {
    if (!ids.has(stem.id)) continue;
    const model = stem.note.modelVel;
    if (model === null || model === undefined) continue;
    values[stem.id] = clampVelocity(model);
  }
  return values;
}
