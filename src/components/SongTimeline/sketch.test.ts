import { expect, it } from 'vitest';

import type { Lane, Note, Pattern } from '../../lib/ipc-types';
import { BUCKETS, density, sketchGradient } from './sketch';

/**
 * The clip's note-density sketch (TASK-070).
 *
 * ⛔ **The meter case is the one worth having.** `patternTicks` in the piano
 * roll shipped with a clip's length computed as bars × four quarters, so it
 * disagreed with the engine for every meter but 4/4 — and the same mistake here
 * bunches a 6/8 clip's notes into the first two thirds of its own sketch and
 * draws the rest as silence. That is a wrong picture that looks like a sparse
 * clip rather than like a bug.
 */

function clip(over: Partial<Pattern> = {}): Pattern {
  return {
    id: 'c',
    part: 'drums',
    artistId: 'trap',
    seed: '1',
    songSeed: '1',
    bars: 1,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'natural_minor',
    lanes: [],
    ppq: 960,
    mood: null,
    loopRegion: null,
    clipRegion: null,
    ...over,
  };
}

function lane(ticks: number[]): { lane: Lane; notes: Note[] } {
  return {
    lane: 'kick',
    notes: ticks.map((startTick) => ({
      startTick,
      lenTicks: 120,
      pitch: 36,
      vel: 100,
      modelVel: null,
      slideToPitch: null,
      articulation: null,
    })),
  };
}

it('an empty clip sketches as nothing at all', () => {
  const levels = density(clip());
  expect(levels).toHaveLength(BUCKETS);
  expect(levels.every((level) => level === 0)).toBe(true);
});

it('a note lands in the bucket its position falls in', () => {
  // One bar of 4/4 is 3840 ticks over 16 buckets — 240 ticks each, which is a
  // 16th note. A note on beat 3 belongs in bucket 8.
  const levels = density(clip({ lanes: [lane([960 * 2])] }));
  expect(levels[8]).toBe(1);
  expect(levels.filter((level) => level > 0)).toHaveLength(1);
});

it('a note at the very end of the clip falls outside it, and is not drawn', () => {
  // ⚠ The bucket index for `startTick === ticks` is `BUCKETS`, which is past the
  // end — so `columnDensity` skips it. That is unreachable in the app rather
  // than a gap: `clampNote` keeps every note's onset strictly inside the clip,
  // and a clip's own length is what defines where 'the end' is.
  const levels = density(clip({ lanes: [lane([960 * 4])] }));
  expect(levels.every((level) => level === 0)).toBe(true);
});

it('density is relative to the clip’s own busiest bucket', () => {
  // A chord pad holding four long notes would otherwise sketch as almost
  // nothing beside a hi-hat lane, and the two are drawn side by side.
  const levels = density(clip({ lanes: [lane([0, 0, 0, 960])] }));
  expect(levels[0]).toBe(1);
  expect(levels[4]).toBeCloseTo(1 / 3);
});

it('a clip in another meter fills its own width', () => {
  // ⛔ 6/8: a bar is three quarter notes, not four. Computed as bars × four
  // quarters the span would be 3840 rather than 2880, and a note in the last
  // eighth would land in bucket 12 of 16 — three columns of phantom silence at
  // the end of every clip.
  const sixEight = clip({ timeSigNum: 6, timeSigDen: 8, lanes: [lane([960 * 3 - 1])] });
  const levels = density(sixEight);
  expect(levels[BUCKETS - 1]).toBe(1);
});

it('the gradient has one hard-edged stop per bucket', () => {
  const css = sketchGradient(density(clip({ lanes: [lane([0])] })));
  expect(css.startsWith('linear-gradient(to right,')).toBe(true);
  // Two positions per stop — the start and the end of its band — so nothing
  // blends across a boundary the sketch cannot actually resolve.
  expect(css.match(/rgba\(/g)).toHaveLength(BUCKETS);
  // An empty bucket is fully transparent rather than merely faint, so a bar
  // with nothing in it reads as empty instead of quiet.
  expect(css).toContain('rgba(255, 255, 255, 0.000)');
});

it('an empty level list asks for no background at all', () => {
  expect(sketchGradient([])).toBe('none');
});
