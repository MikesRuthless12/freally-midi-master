import { describe, expect, it } from 'vitest';

import {
  accelerate,
  colourAt,
  CYAN,
  durationFor,
  ease,
  ignition,
  MAX_DURATION_MS,
  MIN_DURATION_MS,
  progress,
  SETTLE_MS,
  sweepAt,
  VIOLET,
} from './ripple';

describe('ease', () => {
  it('starts at 0, ends at 1, and never leaves the range', () => {
    // A curve that overshoots draws the sweep off the canvas; one that never
    // reaches 1 leaves the last column dark forever.
    expect(ease(0)).toBe(0);
    expect(ease(1)).toBe(1);
    for (const t of [-5, -0.1, 0.25, 0.5, 0.75, 1.1, 99]) {
      expect(ease(t)).toBeGreaterThanOrEqual(0);
      expect(ease(t)).toBeLessThanOrEqual(1);
    }
  });

  it('eases out — more distance covered early than late', () => {
    expect(ease(0.25)).toBeGreaterThan(0.25);
    expect(ease(0.75) - ease(0.5)).toBeLessThan(ease(0.25) - ease(0));
  });
});

describe('progress', () => {
  it('clamps rather than running off either end', () => {
    // A backgrounded tab delivers one frame hours late; a clock that stepped
    // delivers one before the start.
    expect(progress(-100, 400)).toBe(0);
    expect(progress(0, 400)).toBe(0);
    expect(progress(200, 400)).toBe(0.5);
    expect(progress(4_000_000, 400)).toBe(1);
  });

  it('treats a zero duration as finished rather than dividing by it', () => {
    // Infinity or NaN here draws nothing and never clears the canvas.
    expect(progress(10, 0)).toBe(1);
    expect(progress(10, -5)).toBe(1);
  });
});

describe('sweepAt', () => {
  it('crosses the whole width exactly once', () => {
    expect(sweepAt(0, 400)).toBe(0);
    expect(sweepAt(400, 400)).toBe(1);
    // Monotonic: a sweep that goes backwards reads as a glitch.
    let previous = -1;
    for (let ms = 0; ms <= 400; ms += 10) {
      const at = sweepAt(ms, 400);
      expect(at).toBeGreaterThanOrEqual(previous);
      previous = at;
    }
  });
});

describe('ignition', () => {
  it('leaves a cell dark until the sweep reaches it', () => {
    expect(ignition(0.8, 0.5, 400)).toBe(0);
  });

  it('lights a cell fully as the sweep passes', () => {
    expect(ignition(0.5, 0.5, 400)).toBe(1);
  });

  it('decays to nothing over the settle time', () => {
    // The decay is what makes cells look like they ignite rather than like a
    // bar wiping across the grid.
    const duration = 400;
    // Half the settle time behind the sweep.
    const halfSettle = SETTLE_MS / 2 / duration;
    const mid = ignition(0.5 - halfSettle, 0.5, duration);
    expect(mid).toBeGreaterThan(0.4);
    expect(mid).toBeLessThan(0.6);

    // Fully settled: dark again.
    const fullSettle = SETTLE_MS / duration;
    expect(ignition(0.5 - fullSettle, 0.5, duration)).toBe(0);
    expect(ignition(0, 1, duration)).toBe(0);
  });
});

describe('colourAt', () => {
  it('runs violet to cyan and stays inside both ends', () => {
    expect(colourAt(0)).toEqual(VIOLET);
    expect(colourAt(1)).toEqual(CYAN);
    expect(colourAt(-1)).toEqual(VIOLET);
    expect(colourAt(2)).toEqual(CYAN);

    const mid = colourAt(0.5);
    // Between the two on every channel, which a wrong lerp would not be.
    expect(mid.g).toBeGreaterThan(VIOLET.g);
    expect(mid.g).toBeLessThan(CYAN.g);
    expect(mid.r).toBeLessThan(VIOLET.r);
    expect(mid.r).toBeGreaterThan(CYAN.r);
  });
});

describe('durationFor', () => {
  it('outlasts a fast generation and never becomes the wait itself', () => {
    // FR-017's whole point: the animation masks the latency. A 12 ms
    // generation still gets the floor, and a slow one is never padded past
    // the ceiling.
    expect(durationFor(12)).toBe(MIN_DURATION_MS);
    expect(durationFor(0)).toBe(MIN_DURATION_MS);
    expect(durationFor(500)).toBe(500);
    expect(durationFor(5_000)).toBe(MAX_DURATION_MS);
  });
});

describe('accelerate', () => {
  it('compresses what is left rather than snapping to the end', () => {
    // Truncating would jump the sweep to the finish — the exact discontinuity
    // the ripple exists to avoid.
    const shortened = accelerate(100, 800);
    expect(shortened).toBeGreaterThan(100);
    expect(shortened).toBeLessThan(800);
  });

  it('never lands before the time already spent', () => {
    // A duration shorter than the elapsed time drives progress past 1 and the
    // landing frame never draws.
    expect(accelerate(700, 800)).toBeGreaterThanOrEqual(700);
    expect(accelerate(900, 800)).toBeGreaterThanOrEqual(900);
  });
});
