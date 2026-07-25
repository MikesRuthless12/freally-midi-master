/**
 * The generation ripple, as arithmetic (FR-017, TASK-029).
 *
 * Kept apart from the component because this is the part that can be wrong in
 * a way nobody sees: an easing curve that never reaches 1, a sweep that leaves
 * the canvas before the last cell ignites, an acceleration that runs the clock
 * backwards. All of it is pure, so it is tested rather than watched.
 *
 * What it is for: generation takes tens of milliseconds and a beat appearing
 * instantly reads as nothing having happened. The ripple spends 300–800 ms
 * saying "this was made", and the cells igniting as it passes are the notes
 * landing.
 */

/** FR-017's window. Under 300 ms nobody registers it; over 800 ms it is a wait. */
export const MIN_DURATION_MS = 300;
export const MAX_DURATION_MS = 800;

/** How long a cell stays lit after the sweep passes it. */
export const SETTLE_MS = 200;

/** The reduced-motion path: a crossfade, no sweep, no ignition. */
export const CROSSFADE_MS = 150;

/** Violet at the leading edge, cyan behind it — the product's two colours. */
export const VIOLET = { r: 0xa8, g: 0x55, b: 0xf7 };
export const CYAN = { r: 0x22, g: 0xd3, b: 0xee };

export type Rgb = { r: number; g: number; b: number };

/**
 * Ease-out cubic: fast away from the click, settling into the landing.
 *
 * Clamped at both ends, because a frame can arrive before the start time (a
 * clock that stepped) or long after the end (a backgrounded tab), and either
 * one outside 0–1 draws a sweep somewhere off the canvas.
 */
export function ease(t: number): number {
  const clamped = Math.min(1, Math.max(0, t));
  return 1 - Math.pow(1 - clamped, 3);
}

/**
 * How far through the animation we are, 0–1.
 *
 * `duration` of 0 or less means "already finished" rather than a division by
 * zero producing `Infinity` or `NaN` — both of which draw nothing at all and
 * leave the canvas up forever.
 */
export function progress(elapsedMs: number, durationMs: number): number {
  if (durationMs <= 0) return 1;
  return Math.min(1, Math.max(0, elapsedMs / durationMs));
}

/** The sweep front's position across the width, 0–1. */
export function sweepAt(elapsedMs: number, durationMs: number): number {
  return ease(progress(elapsedMs, durationMs));
}

/**
 * How lit a cell at `x` (0–1 across the width) is, 0–1.
 *
 * Zero until the sweep reaches it, full as it passes, then decaying over
 * `SETTLE_MS` — which is what makes cells look like they *ignite* rather than
 * like a bar wiping across them.
 */
export function ignition(x: number, sweep: number, durationMs: number): number {
  if (x > sweep) return 0;
  // How long ago, in milliseconds, the sweep passed this column.
  const sinceMs = (sweep - x) * durationMs;
  if (sinceMs >= SETTLE_MS) return 0;
  return 1 - sinceMs / SETTLE_MS;
}

/** Violet → cyan along the sweep. */
export function colourAt(t: number): Rgb {
  const clamped = Math.min(1, Math.max(0, t));
  return {
    r: Math.round(VIOLET.r + (CYAN.r - VIOLET.r) * clamped),
    g: Math.round(VIOLET.g + (CYAN.g - VIOLET.g) * clamped),
    b: Math.round(VIOLET.b + (CYAN.b - VIOLET.b) * clamped),
  };
}

/**
 * The duration to run for, given how long generation actually took.
 *
 * FR-017: the ripple masks the latency, so it must outlast a fast generation
 * (hence the floor) without ever becoming the thing being waited for (hence
 * the ceiling). A slow generation is *not* padded — the animation is already
 * running while it works.
 */
export function durationFor(generationMs: number): number {
  return Math.min(MAX_DURATION_MS, Math.max(MIN_DURATION_MS, generationMs));
}

/**
 * Compress what is left when the pattern lands early.
 *
 * "If generation finishes first, the ripple accelerates to landing" — so the
 * remaining time is scaled, not truncated. Truncating would snap the sweep to
 * the end, which is the jump the animation exists to avoid.
 *
 * Returns the new duration. Never shorter than the elapsed time, or progress
 * would jump backwards past 1 and the final frame would never draw.
 */
export function accelerate(elapsedMs: number, durationMs: number, factor = 0.45): number {
  const remaining = Math.max(0, durationMs - elapsedMs);
  return elapsedMs + remaining * factor;
}
