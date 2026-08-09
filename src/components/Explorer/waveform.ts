/**
 * Turning the peaks `explorer_waveform` returns into something drawable.
 *
 * A leaf module rather than a function beside the component, for the same
 * reason `PianoRoll/geometry.ts` and `SongTimeline/clips.ts` are: it is pure
 * arithmetic with no React in it, so it can be tested directly — and a
 * component file that also exports helpers costs Fast Refresh.
 */

/** The drawing box the peaks are mapped into. Unitless — the SVG scales. */
export const VIEW_W = 800;
export const VIEW_H = 100;
/** Leaves a hairline of margin so a full-scale sample is not clipped flat. */
const AMPLITUDE = 48;

/**
 * The peaks as one filled outline: the maxima left-to-right, then the minima
 * back again.
 *
 * ⚠ **Both bounds, which is why `explorer::waveform` returns a pair.** A single
 * amplitude per column draws a half-waveform that reads as a DC-offset
 * recording rather than as audio — the Rust side has a test asserting exactly
 * that, and drawing only one of them here is what would have made that test
 * prove nothing.
 *
 * ⚠ **Clamped to -1..1.** The peaks come out of a decoder, and a clipped or
 * float-format file really can exceed full scale; unclamped it draws outside
 * the box and the SVG crops the loudest part of the sample flat.
 */
export function outlineOf(peaks: readonly [number, number][]): string {
  if (peaks.length === 0) return '';
  const x = (index: number) => (index * VIEW_W) / Math.max(1, peaks.length - 1);
  const y = (amplitude: number) =>
    VIEW_H / 2 - Math.max(-1, Math.min(1, amplitude)) * AMPLITUDE;

  const tops = peaks.map(([, high], index) => `${x(index).toFixed(2)},${y(high).toFixed(2)}`);
  const bottoms = peaks
    .map(([low], index) => `${x(index).toFixed(2)},${y(low).toFixed(2)}`)
    .reverse();
  return `M${tops.join('L')}L${bottoms.join('L')}Z`;
}
