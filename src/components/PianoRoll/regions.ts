/**
 * What the ruler strip holds, and what a pointer in it grabbed (TASK-041E).
 *
 * Pure and separate from the canvas for the same reason `geometry.ts` and
 * `velocity.ts` are: the strip draws four different affordances within about
 * twenty pixels of each other, and the only way the hit test and the drawing
 * can agree about which one is under the pointer is for both to ask this.
 */

import type { Pattern, Region } from '../../lib/ipc-types';
import { patternTicks } from './notes';

/** The strip's height in CSS pixels. */
export const RULER_HEIGHT = 24;

/** How near a handle a pointer has to be to have meant it. */
export const HANDLE_PX = 6;

/**
 * The band the stretch handles live in.
 *
 * ⛔ **Below the brace, not beside it.** The loop's end and a selection's outer
 * edge are frequently the same tick — a producer loops what they selected — so
 * splitting them by x alone would make one of the two ungrabbable exactly when
 * both matter. Splitting by y makes both reachable at the same tick.
 */
export const STRETCH_BAND_TOP = RULER_HEIGHT / 2;

/** What a pointer in the strip is holding. */
export type Grip =
  | { kind: 'loop'; edge: 'from' | 'to' }
  | { kind: 'clip'; edge: 'from' | 'to' }
  | { kind: 'stretch'; edge: 'from' | 'to' }
  /** Empty ruler: the drag draws a new loop region. */
  | { kind: 'new' };

/** The loop region a pattern is actually playing — the whole clip by default. */
export function loopOf(pattern: Pattern): Region {
  const whole = { fromTick: 0, toTick: patternTicks(pattern) };
  const set = pattern.loopRegion;
  if (!set || set.toTick <= set.fromTick) return whole;
  return set;
}

/** The clip's own start and end, which default to the whole thing as well. */
export function clipOf(pattern: Pattern): Region {
  const whole = { fromTick: 0, toTick: patternTicks(pattern) };
  const set = pattern.clipRegion;
  if (!set || set.toTick <= set.fromTick) return whole;
  return set;
}

/**
 * Decide what a pointer at `(x, y)` grabbed.
 *
 * `xOf` maps a tick to the same x the strip drew it at, so this cannot drift
 * from the drawing. `stretch` is the selection's outer edges, or `null` when
 * fewer than two notes are selected and there is nothing to stretch.
 */
export function gripAt(
  x: number,
  y: number,
  loop: Region,
  clip: Region,
  stretch: Region | null,
  xOf: (tick: number) => number,
): Grip {
  const near = (tick: number) => Math.abs(x - xOf(tick)) <= HANDLE_PX;

  if (stretch !== null && y >= STRETCH_BAND_TOP) {
    if (near(stretch.fromTick)) return { kind: 'stretch', edge: 'from' };
    if (near(stretch.toTick)) return { kind: 'stretch', edge: 'to' };
  }

  if (near(loop.fromTick)) return { kind: 'loop', edge: 'from' };
  if (near(loop.toTick)) return { kind: 'loop', edge: 'to' };

  // ⛔ The clip markers are tested *after* the loop's, because until someone
  // drags a brace the two sit on exactly the same ticks — and the brace is the
  // one a producer reaches for first.
  if (near(clip.fromTick)) return { kind: 'clip', edge: 'from' };
  if (near(clip.toTick)) return { kind: 'clip', edge: 'to' };

  return { kind: 'new' };
}

/**
 * Move one edge of a region, keeping it a region.
 *
 * ⛔ **The dragged edge is stopped a step short of the other**, rather than
 * allowed to cross it. A brace dragged inside out is refused downstream — see
 * `Region::valid` — so letting it invert on screen would show a loop the
 * transport is quietly ignoring.
 */
export function moveEdge(
  region: Region,
  edge: 'from' | 'to',
  tick: number,
  step: number,
  limit: number,
): Region {
  const gap = Math.max(1, step);
  const at = Math.min(Math.max(0, Math.round(tick)), limit);
  // ⛔ **Both ends floored at zero, and the `from` end is the one that mattered.**
  // Clamping only `at` was not enough: `toTick - gap` is itself negative once a
  // brace has been dragged shorter than the current snap step, so dragging the
  // left handle produced `fromTick: -900`. `Region.from_tick` is a `u32`, so
  // that payload is rejected by *both* `arm_pattern` and `save_session_state` —
  // and both rejections are swallowed. The session then silently stopped
  // persisting anything at all: artist, seed, pins, mutes, the edited clip.
  return edge === 'from'
    ? { ...region, fromTick: Math.max(0, Math.min(at, region.toTick - gap)) }
    : { ...region, toTick: Math.min(limit, Math.max(at, region.fromTick + gap)) };
}

/** A region from two ticks dragged in either order. */
export function regionBetween(a: number, b: number, step: number, limit: number): Region {
  const gap = Math.max(1, step);
  const from = Math.min(Math.max(0, Math.round(Math.min(a, b))), limit);
  const to = Math.min(Math.max(from + gap, Math.round(Math.max(a, b))), limit);
  return { fromTick: from, toTick: to };
}

/**
 * The factor a stretch handle drag is asking for.
 *
 * The anchor is the edge that is *not* being dragged, so pulling the right
 * handle grows the selection to the right and pulling the left one grows it to
 * the left — which is what a handle on that side has to mean.
 */
export function stretchFactor(span: Region, tick: number): number | null {
  const length = span.toTick - span.fromTick;
  if (length <= 0) return null;
  // ⛔ **Both handles measure from the selection's *start*, because that is what
  // `stretch` anchors at.** Measuring the left handle against the right end
  // read correctly as prose and was wrong in practice: `stretch` always scales
  // rightward from `span.from`, so dragging the left handle to the middle
  // produced a factor of 0.5 and a block at 0–960 — the left handle moving the
  // right edge. Until `stretch` can anchor at either end, the honest thing is
  // for both handles to mean the same gesture.
  const wanted = tick - span.fromTick;
  if (wanted <= 0) return null;
  return wanted / length;
}
