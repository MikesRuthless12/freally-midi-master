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

/**
 * The envelope as a polyline: attack up, decay to sustain, hold, release down.
 */
export function envelopePath(adsr: Adsr): string {
  const x = (ms: number) => (clamp(ms, 0, ENV_SPAN_MS) / ENV_SPAN_MS) * (ENV_W / 3);
  // dB to a height. ⚠ Linear in dB rather than in amplitude, because the handle
  // is dragged against a dB readout — a graph in amplitude would put −6 dB
  // halfway up and −36 dB on the floor, and the drag would not match the number.
  const y = (db: number) => ENV_H - ((clamp(db, MIN_DB, 0) - MIN_DB) / -MIN_DB) * ENV_H;

  const attackAt = x(adsr.attackMs);
  const decayAt = attackAt + x(adsr.decayMs);
  const sustainY = y(adsr.sustainDb);
  // The sustain leg is drawn at a fixed width: it is a *level*, not a duration —
  // how long it lasts is the note's length, which this graph does not know.
  const releaseAt = decayAt + ENV_W / 4;
  const endAt = releaseAt + x(adsr.releaseMs);

  return [
    `M0,${ENV_H}`,
    `L${attackAt.toFixed(1)},0`,
    `L${decayAt.toFixed(1)},${sustainY.toFixed(1)}`,
    `L${releaseAt.toFixed(1)},${sustainY.toFixed(1)}`,
    `L${endAt.toFixed(1)},${ENV_H}`,
  ].join('');
}
