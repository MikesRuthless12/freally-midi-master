/**
 * The arrangement's ruler and grid (TASK-063B).
 *
 * The requirement these pin is "the grid resolution follows the zoom rather
 * than staying fixed", which is the one thing a screenshot of a timeline cannot
 * tell you — a fixed grid looks perfectly reasonable at exactly one zoom level.
 */

import { describe, expect, it } from 'vitest';

import {
  MAX_ZOOM,
  MIN_ZOOM,
  barLabel,
  barToSeconds,
  barToX,
  formatTime,
  gridFor,
  xToBar,
  zoomIn,
  zoomOut,
} from './geometry';

describe('bar and pixel conversion', () => {
  it('round-trips', () => {
    const view = { zoom: 24, scrollBar: 8 };
    for (const bar of [0, 8, 12.5, 64]) {
      expect(xToBar(barToX(bar, view), view)).toBeCloseTo(bar, 6);
    }
  });

  it('puts the scrolled-to bar at the left edge', () => {
    expect(barToX(8, { zoom: 24, scrollBar: 8 })).toBe(0);
  });
});

describe('zoom', () => {
  it('steps geometrically and stops at both ends', () => {
    // A linear step is useless at one end or the other: +8 px/bar doubles the
    // scale at 8 and is invisible at 200.
    expect(zoomIn(20)).toBeGreaterThan(20);
    expect(zoomOut(20)).toBeLessThan(20);
    expect(zoomIn(MAX_ZOOM)).toBe(MAX_ZOOM);
    expect(zoomOut(MIN_ZOOM)).toBe(MIN_ZOOM);
  });

  it('never leaves the range however many times it is pressed', () => {
    let zoom = 24;
    for (let i = 0; i < 50; i += 1) zoom = zoomIn(zoom);
    expect(zoom).toBe(MAX_ZOOM);
    for (let i = 0; i < 50; i += 1) zoom = zoomOut(zoom);
    expect(zoom).toBe(MIN_ZOOM);
  });
});

describe('the grid follows the zoom', () => {
  it('draws beats only when a bar is wide enough to hold them', () => {
    // ⛔ The requirement itself. Zoomed in, beats appear; zoomed out they would
    // be an unreadable smear, so they are not drawn at all.
    expect(gridFor({ zoom: 120, scrollBar: 0 }, 4).beatStep).toBe(1);
    expect(gridFor({ zoom: 10, scrollBar: 0 }, 4).beatStep).toBe(0);
  });

  it('thins bar lines out in musical steps as it zooms out', () => {
    const close = gridFor({ zoom: 120, scrollBar: 0 }, 4).barStep;
    const far = gridFor({ zoom: 6, scrollBar: 0 }, 4).barStep;
    expect(close).toBe(1);
    expect(far).toBeGreaterThan(close);
    // Powers of two, because a producer counts in phrases — a step of 3 or 5
    // would put the heavy lines where no section boundary ever falls.
    for (const zoom of [6, 10, 24, 60, 120, 240]) {
      const { barStep, labelStep } = gridFor({ zoom, scrollBar: 0 }, 4);
      expect(Number.isInteger(Math.log2(barStep))).toBe(true);
      expect(Number.isInteger(Math.log2(labelStep))).toBe(true);
      expect(labelStep).toBeGreaterThanOrEqual(barStep);
    }
  });

  it('never draws two lines closer together than they can be read', () => {
    // The property behind the thresholds: whatever the zoom, the drawn spacing
    // stays legible. This is what catches a step rule that stops widening.
    for (let zoom = MIN_ZOOM; zoom <= MAX_ZOOM; zoom += 1) {
      const { barStep, labelStep } = gridFor({ zoom, scrollBar: 0 }, 4);
      expect(zoom * barStep).toBeGreaterThanOrEqual(7);
      expect(zoom * labelStep).toBeGreaterThanOrEqual(44);
    }
  });

  it('reads the beats per bar rather than assuming four', () => {
    // A 6/8 song must not be gridded as 4/4 — the same bug `patternTicks` had.
    const wide = gridFor({ zoom: 40, scrollBar: 0 }, 4).beatStep;
    const narrow = gridFor({ zoom: 40, scrollBar: 0 }, 16).beatStep;
    expect(wide).toBe(1);
    expect(narrow).toBe(0);
  });
});

describe('the ruler', () => {
  it('counts bars from one, the way every DAW does', () => {
    // A producer reading "0" assumes the display is broken.
    expect(barLabel(0)).toBe('1');
    expect(barLabel(15)).toBe('16');
  });

  it('turns bars into wall-clock time', () => {
    // 140 bpm, 4/4: a bar is 4 beats = 60/140*4 seconds.
    expect(barToSeconds(0, 140, 4)).toBe(0);
    expect(barToSeconds(1, 140, 4)).toBeCloseTo(1.714, 3);
    // The pop question the research states outright: is the chorus inside 60 s?
    expect(barToSeconds(28, 140, 4)).toBeCloseTo(48, 1);
  });

  it('survives a tempo that cannot be right', () => {
    // A zero or NaN bpm from a host that has not reported yet must not make the
    // ruler print Infinity.
    for (const bpm of [0, -140, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(Number.isFinite(barToSeconds(4, bpm, 4))).toBe(true);
    }
  });

  it('formats as m:ss with a padded seconds field', () => {
    expect(formatTime(0)).toBe('0:00');
    expect(formatTime(9)).toBe('0:09');
    expect(formatTime(61.7)).toBe('1:01');
    expect(formatTime(-5)).toBe('0:00');
  });
});
