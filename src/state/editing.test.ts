import { beforeEach, describe, expect, it } from 'vitest';

import { DEFAULT_ROW_HEIGHT, MIN_ROW_HEIGHT, MIN_ZOOM, useEditing } from './editing';

/**
 * The two framings, and the invariant they exist for.
 *
 * ⛔⛔ **"All the notes, not most of them"** (Mike, 2026-08-09). `fitTo` was
 * written for the horizontal half of that sentence and shipped without the
 * vertical half, and nothing in the suite could tell: every test asked whether
 * the roll had *scrolled* to the register, and scrolling to a register shows
 * exactly as many semitones as the row height already allowed. The check that
 * catches it is the one below — how many rows *fit*, against how many the clip
 * needs.
 */

const START = useEditing.getState();

beforeEach(() => {
  useEditing.setState({
    zoom: START.zoom,
    rowHeight: START.rowHeight,
    scrollTick: START.scrollTick,
    topPitch: START.topPitch,
  });
});

/** The lowest pitch still on screen, which is what "visible" has to mean. */
function bottomPitch(viewportPx: number): number {
  const { topPitch, rowHeight } = useEditing.getState();
  return topPitch - Math.floor(viewportPx / rowHeight) + 1;
}

/** How far off centre the written range sits, in rows. */
function offCentre(highest: number, lowest: number, viewportPx: number): number {
  const { topPitch } = useEditing.getState();
  const above = topPitch - highest;
  const below = lowest - bottomPitch(viewportPx);
  return Math.abs(above - below);
}

describe('frameTo', () => {
  it('shows a two-octave melody whole, by shrinking the row', () => {
    // 24 semitones needs 26 rows with its air; 400 px of stage at the default
    // 20 px row shows 20. This is the case Mike reported.
    useEditing.getState().frameTo(78, 54, 400);

    const { rowHeight } = useEditing.getState();
    expect(rowHeight).toBeLessThan(DEFAULT_ROW_HEIGHT);
    expect(bottomPitch(400)).toBeLessThanOrEqual(54);
    expect(useEditing.getState().topPitch).toBeGreaterThanOrEqual(78);
  });

  it('leaves a row of air at each end rather than sitting flush', () => {
    useEditing.getState().frameTo(78, 54, 400);

    expect(useEditing.getState().topPitch).toBeGreaterThan(78);
    expect(bottomPitch(400)).toBeLessThanOrEqual(53);
  });

  it('does not blow a three-note bass line up past the default row', () => {
    // 500 px over five rows would be 100 px each. The default is the height a
    // producer expects to open on, and it is the ceiling.
    useEditing.getState().frameTo(38, 36, 500);

    expect(useEditing.getState().rowHeight).toBe(DEFAULT_ROW_HEIGHT);
    expect(bottomPitch(500)).toBeLessThanOrEqual(36);
  });

  it('splits the leftover rows above and below rather than dumping them below', () => {
    // ⛔⛔ The regression this pins. An 808 line of five semitones in a 500 px
    // roll gets 25 rows for a range needing 8, and anchoring `topPitch` at
    // `highest + 1` jammed all five against the ruler with four hundred pixels
    // of empty keyboard underneath.
    useEditing.getState().frameTo(40, 36, 500);
    expect(offCentre(40, 36, 500)).toBeLessThanOrEqual(1);

    // And the same for a range that only just fits, where the clamp does not bite.
    useEditing.getState().frameTo(78, 54, 400);
    expect(offCentre(78, 54, 400)).toBeLessThanOrEqual(1);
  });

  it('comes back to the default row for a narrow clip after a wide one', () => {
    useEditing.getState().frameTo(100, 30, 400);
    expect(useEditing.getState().rowHeight).toBe(MIN_ROW_HEIGHT);

    // ⛔ The regression this pins: clamping the upper end to the *current* row
    // height would leave the roll stuck at 8 px for the rest of the session.
    useEditing.getState().frameTo(62, 60, 400);
    expect(useEditing.getState().rowHeight).toBe(DEFAULT_ROW_HEIGHT);
  });

  it('centres a range too tall to fit even at the smallest row', () => {
    // 98 semitones needs 100 rows; 400 px at the 8 px floor is 50. It cannot be
    // shown whole, so the honest answer is the middle of it — not a top edge
    // with the other half below the fold.
    useEditing.getState().frameTo(110, 12, 400);

    const { rowHeight, topPitch } = useEditing.getState();
    expect(rowHeight).toBe(MIN_ROW_HEIGHT);
    expect(topPitch).toBeLessThan(110);
    expect(topPitch).toBeGreaterThan(61 + 20);
  });

  it('never asks for a pitch MIDI does not have', () => {
    useEditing.getState().frameTo(127, 120, 400);
    expect(useEditing.getState().topPitch).toBe(127);
  });

  it('ignores a viewport that has not been measured yet', () => {
    // The effect reads `clientHeight` off a ref, which is 0 on the first pass
    // in jsdom and in a hidden tab. Dividing by it would set an infinite row.
    useEditing.getState().frameTo(78, 54, 0);
    expect(useEditing.getState().rowHeight).toBe(DEFAULT_ROW_HEIGHT);
  });
});

describe('fitTo', () => {
  it('still fits the horizontal half, and stops at the zoom floor', () => {
    useEditing.getState().fitTo(960 * 4, 960, 800);
    expect(useEditing.getState().zoom).toBeCloseTo((800 * 0.985) / 4);

    useEditing.getState().fitTo(960 * 4000, 960, 800);
    expect(useEditing.getState().zoom).toBe(MIN_ZOOM);
  });
});
