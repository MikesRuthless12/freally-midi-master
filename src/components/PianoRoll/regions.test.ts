import { describe, expect, it } from 'vitest';

import type { Pattern, Region } from '../../lib/ipc-types';
import {
  HANDLE_PX,
  STRETCH_BAND_TOP,
  clipOf,
  gripAt,
  loopOf,
  moveEdge,
  regionBetween,
  stretchFactor,
} from './regions';

const PPQ = 960;
const BAR = PPQ * 4;

function clip(over: Partial<Pattern> = {}): Pattern {
  return {
    id: 'x',
    part: 'melody',
    artistId: 'a',
    seed: '1',
    bars: 4,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'major',
    ppq: PPQ,
    lanes: [],
    ...over,
  };
}

/** One pixel per 100 ticks, so a tick is easy to read as an x. */
const xOf = (tick: number) => tick / 100;

describe('what the strip is showing', () => {
  it('is the whole clip until someone drags a brace', () => {
    expect(loopOf(clip())).toEqual({ fromTick: 0, toTick: BAR * 4 });
    expect(clipOf(clip())).toEqual({ fromTick: 0, toTick: BAR * 4 });
  });

  it('is the region the clip carries once it has one', () => {
    const region: Region = { fromTick: BAR, toTick: BAR * 3 };
    expect(loopOf(clip({ loopRegion: region }))).toEqual(region);
  });

  it('ignores an inverted region rather than drawing a backwards brace', () => {
    // ⛔ The transport refuses it too — `Region::valid` — so drawing it would
    // show a loop nothing is playing.
    const region: Region = { fromTick: BAR * 3, toTick: BAR };
    expect(loopOf(clip({ loopRegion: region }))).toEqual({ fromTick: 0, toTick: BAR * 4 });
  });
});

describe('what a pointer in the strip grabbed', () => {
  const loop: Region = { fromTick: BAR, toTick: BAR * 3 };
  const clipRegion: Region = { fromTick: 0, toTick: BAR * 4 };

  it('takes the loop handle it is nearest', () => {
    expect(gripAt(xOf(BAR), 4, loop, clipRegion, null, xOf)).toEqual({
      kind: 'loop',
      edge: 'from',
    });
    expect(gripAt(xOf(BAR * 3), 4, loop, clipRegion, null, xOf)).toEqual({
      kind: 'loop',
      edge: 'to',
    });
  });

  it('takes the clip marker when no loop handle is near', () => {
    expect(gripAt(xOf(BAR * 4), 4, loop, clipRegion, null, xOf)).toEqual({
      kind: 'clip',
      edge: 'to',
    });
  });

  it('draws a new region from empty ruler', () => {
    expect(gripAt(xOf(BAR * 2), 4, loop, clipRegion, null, xOf)).toEqual({ kind: 'new' });
  });

  it('reaches a stretch handle under the brace at the very same tick', () => {
    // ⛔ The case that forced the two bands: a producer loops what they
    // selected, so both handles sit on one tick and x alone cannot tell them
    // apart. The lower half is the stretch handle's.
    const selection: Region = { fromTick: BAR, toTick: BAR * 3 };
    expect(gripAt(xOf(BAR), STRETCH_BAND_TOP + 1, loop, clipRegion, selection, xOf)).toEqual({
      kind: 'stretch',
      edge: 'from',
    });
    expect(gripAt(xOf(BAR), STRETCH_BAND_TOP - 1, loop, clipRegion, selection, xOf)).toEqual({
      kind: 'loop',
      edge: 'from',
    });
  });

  it('only grabs a handle the pointer is actually near', () => {
    expect(gripAt(xOf(BAR) + HANDLE_PX + 1, 4, loop, clipRegion, null, xOf)).toEqual({
      kind: 'new',
    });
  });
});

describe('dragging an edge', () => {
  const region: Region = { fromTick: BAR, toTick: BAR * 3 };

  it('moves the edge that was grabbed and leaves the other alone', () => {
    expect(moveEdge(region, 'from', BAR * 2, 240, BAR * 4)).toEqual({
      fromTick: BAR * 2,
      toTick: BAR * 3,
    });
  });

  it('stops a step short rather than turning the region inside out', () => {
    expect(moveEdge(region, 'from', BAR * 4, 240, BAR * 4).fromTick).toBe(BAR * 3 - 240);
    expect(moveEdge(region, 'to', 0, 240, BAR * 4).toTick).toBe(BAR + 240);
  });

  it('cannot be dragged outside the clip', () => {
    expect(moveEdge(region, 'to', BAR * 99, 240, BAR * 4).toTick).toBe(BAR * 4);
    expect(moveEdge(region, 'from', -500, 240, BAR * 4).fromTick).toBe(0);
  });
});

describe('drawing a region from a drag', () => {
  it('works in either direction', () => {
    expect(regionBetween(BAR * 3, BAR, 240, BAR * 4)).toEqual({
      fromTick: BAR,
      toTick: BAR * 3,
    });
  });

  it('is never empty, however small the drag was', () => {
    const tiny = regionBetween(BAR, BAR + 3, 240, BAR * 4);
    expect(tiny.toTick - tiny.fromTick).toBeGreaterThanOrEqual(240);
  });
});

describe('the stretch handles', () => {
  const span: Region = { fromTick: BAR, toTick: BAR * 3 };

  it('doubles the selection when the right handle is pulled to twice its length', () => {
    expect(stretchFactor(span, 'to', BAR * 5)).toBe(2);
  });

  it('measures from the other edge when the left handle is pulled', () => {
    expect(stretchFactor(span, 'from', BAR - BAR * 2)).toBe(2);
  });

  it('refuses a drag past the anchor rather than inverting the selection', () => {
    expect(stretchFactor(span, 'to', BAR - 1)).toBeNull();
    expect(stretchFactor({ fromTick: 0, toTick: 0 }, 'to', BAR)).toBeNull();
  });
});
