/**
 * Drawing a pad's ADSR (TASK-164).
 *
 * A leaf module rather than a function beside the component, for the same
 * reason `Explorer/waveform.ts` and `PianoRoll/geometry.ts` are: it is pure
 * arithmetic with no React in it, so it can be tested directly — and a
 * component file that also exports helpers costs Fast Refresh.
 */

import type { Adsr } from '../../state/kit';

/** Where the envelope is drawn. Unitless — the SVG scales to the panel. */
export const ENV_W = 320;
export const ENV_H = 96;

/**
 * The longest stage the graph draws at full width, in ms.
 *
 * ⚠ **A drawing bound, not a limit on the value.** A ten-second release is a
 * legitimate thing to ask for and the sampler honours it; past this the handle
 * simply stops travelling, because a graph that rescaled itself as you dragged
 * would move the handle you are holding away from the pointer.
 */
export const ENV_SPAN_MS = 2_000;

/** dB floor for the sustain handle, matching `pad_tweaks::MIN_GAIN_DB`. */
export const MIN_DB = -60;

function clamp(value: number, low: number, high: number): number {
  return Math.max(low, Math.min(high, value));
}

/** How wide one stage's full travel is, in drawing units. */
const STAGE_W = ENV_W / 3;

/**
 * The sustain leg's drawn width.
 *
 * ⚠ Fixed, because sustain is a *level* rather than a duration — how long it
 * lasts is the note's length, which this graph does not know.
 */
const SUSTAIN_W = ENV_W / 4;

/** A stage duration to a horizontal distance. */
function spanOf(ms: number): number {
  return (clamp(ms, 0, ENV_SPAN_MS) / ENV_SPAN_MS) * STAGE_W;
}

/**
 * A distance back to a duration — what a horizontal drag means.
 *
 * ⚠ The exact inverse of [`spanOf`], so a handle dragged to a point and read
 * back lands where it was put.
 */
export function msOf(span: number): number {
  return clamp((span / STAGE_W) * ENV_SPAN_MS, 0, ENV_SPAN_MS);
}

/**
 * A level to a height.
 *
 * ⚠ Linear in dB rather than in amplitude, because the handle is dragged
 * against a dB readout — a graph in amplitude would put −6 dB halfway up and
 * −36 dB on the floor, and the drag would not match the number.
 */
function heightOf(db: number): number {
  return ENV_H - ((clamp(db, MIN_DB, 0) - MIN_DB) / -MIN_DB) * ENV_H;
}

/** A height back to a level — what a vertical drag means. */
export function dbOf(height: number): number {
  return clamp(((ENV_H - height) / ENV_H) * -MIN_DB + MIN_DB, MIN_DB, 0);
}

/** One draggable corner of the envelope. */
export type EnvelopePoint = { x: number; y: number };

/** All four of them, keyed by stage. */
export type EnvelopeCorners = Record<'a' | 'd' | 's' | 'r', EnvelopePoint>;

/**
 * The four corners a producer can take hold of (TASK-055).
 *
 * ⛔ **Shared with [`envelopePath`] rather than computed twice.** The handles
 * have to sit exactly on the line they move; two spellings of the same
 * arithmetic is how a handle ends up a few pixels off the curve it belongs to,
 * and the drift would be invisible until someone changed one of them.
 */
export function envelopePoints(adsr: Adsr): EnvelopeCorners {
  const attackAt = spanOf(adsr.attackMs);
  const decayAt = attackAt + spanOf(adsr.decayMs);
  const sustainY = heightOf(adsr.sustainDb);
  const releaseAt = decayAt + SUSTAIN_W;
  return {
    a: { x: attackAt, y: 0 },
    d: { x: decayAt, y: sustainY },
    s: { x: releaseAt, y: sustainY },
    r: { x: releaseAt + spanOf(adsr.releaseMs), y: ENV_H },
  };
}

/**
 * The envelope as a polyline: attack up, decay to sustain, hold, release down.
 */
export function envelopePath(adsr: Adsr): string {
  const { a, d, s, r } = envelopePoints(adsr);
  return [
    `M0,${ENV_H}`,
    `L${a.x.toFixed(1)},${a.y.toFixed(1)}`,
    `L${d.x.toFixed(1)},${d.y.toFixed(1)}`,
    `L${s.x.toFixed(1)},${s.y.toFixed(1)}`,
    `L${r.x.toFixed(1)},${r.y.toFixed(1)}`,
  ].join('');
}
