import { expect, it } from 'vitest';

import type { Lane, Note, Pattern } from '../../lib/ipc-types';
import { CLIP_H, CLIP_W, clipFormat, notesPath } from './clipArt';

/**
 * What a clip looks like, and what it can be handed over as (TASK-142).
 *
 * ⛔ **The meter case is the one worth having, and it is inherited from the
 * note-density sketch this replaced.** `patternTicks` in the piano roll shipped
 * with a clip's length computed as bars × four quarters, so it disagreed with
 * the engine for every meter but 4/4 — and the same mistake here bunches a 6/8
 * clip's notes into the first two thirds of its own width and leaves the rest
 * empty. That is a wrong picture that looks like a sparse clip rather than like
 * a bug, which is exactly the class of thing this file exists for.
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

function lane(
  ticks: number[],
  laneName: Lane = 'kick',
  pitch = 36,
): { lane: Lane; notes: Note[] } {
  return {
    lane: laneName,
    notes: ticks.map((startTick) => ({
      startTick,
      lenTicks: 120,
      pitch,
      vel: 100,
      modelVel: null,
      slideToPitch: null,
      articulation: null,
    })),
  };
}

/** Every `M x y` origin in a path, as `[x, y]` pairs. */
function origins(path: string): [number, number][] {
  return [...path.matchAll(/M([\d.]+) ([\d.]+)/g)].map((m) => [Number(m[1]), Number(m[2])]);
}

it('an empty clip draws nothing at all', () => {
  expect(notesPath(clip(), 0)).toBe('');
  expect(notesPath(clip({ lanes: [lane([])] }), 0)).toBe('');
  expect(notesPath(null, 0)).toBe('');
});

it('a note lands at the fraction of the clip its tick falls on', () => {
  // ⛔ **The whole point of drawing notes rather than a density smear**: the
  // position has to be the note's own, so two clips with the same note *count*
  // and different placements look different.
  const path = notesPath(clip({ lanes: [lane([0, 960, 1920, 2880])] }), 0);
  const xs = origins(path).map(([x]) => x);
  expect(xs).toEqual([0, 25, 50, 75]);
});

it('a clip in another meter fills its own width', () => {
  // ⛔ 6/8 is three quarter notes to the bar, not six — so the last note of a
  // 6/8 bar has to land at the right edge. Computed as bars × four quarters it
  // sits at 66% and the clip draws a third of itself empty.
  const sixEight = clip({ timeSigNum: 6, timeSigDen: 8, lanes: [lane([960 * 3 - 240])] });
  const [[x]] = origins(notesPath(sixEight, 0));
  expect(x).toBeGreaterThan(CLIP_W * 0.85);
  expect(x).toBeLessThan(CLIP_W);
});

it('a note at or past the end of the clip is not drawn', () => {
  // Its x would be at or beyond the right edge, so it would either draw a
  // zero-width sliver on the border or bleed into the next repeat.
  expect(notesPath(clip({ lanes: [lane([3840])] }), 0)).toBe('');
});

it('pitch runs bottom to top, and a flat line stays flat', () => {
  // ⚠ **The floor on the pitch span is what makes this true.** Scaled to the
  // notes' own range, a bassline sitting on two adjacent semitones would draw
  // one at the very bottom and one at the very top — a huge leap on screen for
  // a line that barely moves.
  const wide = notesPath(
    clip({ lanes: [lane([0], 'melody', 48), lane([960], 'melody', 72)] }),
    0,
  );
  const [low, high] = origins(wide).map(([, y]) => y);
  expect(low).toBeGreaterThan(high);

  const narrow = notesPath(
    clip({ lanes: [lane([0], 'melody', 60), lane([960], 'melody', 61)] }),
    0,
  );
  const ys = origins(narrow).map(([, y]) => y);
  expect(Math.abs(ys[0] - ys[1])).toBeLessThan(CLIP_H * 0.2);
});

it('every note is drawn inside the box, whatever its pitch', () => {
  // The SVG has no overflow to spare: a y outside 0..CLIP_H is cropped, so a
  // note drawn there is a note the producer cannot see.
  const path = notesPath(
    clip({ lanes: [lane([0], 'melody', 0), lane([960], 'melody', 127)] }),
    0,
  );
  for (const [x, y] of origins(path)) {
    expect(x).toBeGreaterThanOrEqual(0);
    expect(x).toBeLessThan(CLIP_W);
    expect(y).toBeGreaterThanOrEqual(0);
    expect(y).toBeLessThanOrEqual(CLIP_H);
  }
});

it('a sixteenth is still wide enough to see', () => {
  // ⚠ Scaled honestly a hi-hat in a four-bar clip is a fraction of a percent
  // wide and the clip draws empty — which is the failure the density sketch was
  // reaching for and solved by not drawing notes at all.
  const path = notesPath(clip({ bars: 4, lanes: [lane([0])] }), 0);
  const [width] = [...path.matchAll(/h([\d.]+)/g)].map((m) => Number(m[1]));
  expect(width).toBeGreaterThanOrEqual(1);
});

it('a resized clip draws only the bars its loop actually plays', () => {
  // ⛔⛔ **The timeline tiles this path once per repeat**, so scaling it to the
  // pattern's own length squeezed all four bars into each one-bar tile — the
  // view drawing notes the engine refuses to play, which is the exact
  // disagreement between screen, transport and export this whole module is
  // supposed to end. `SectionTiling::sounds` drops anything at or past the loop
  // point; so does this.
  const four = clip({ bars: 4, lanes: [lane([0, 3840, 7680, 11520])] });

  // The whole clip: four notes, one per bar, evenly spread.
  expect(origins(notesPath(four, 4)).map(([x]) => x)).toEqual([0, 25, 50, 75]);

  // Looped on one bar: only the note inside that bar, filling the tile.
  expect(origins(notesPath(four, 1)).map(([x]) => x)).toEqual([0]);

  // Looped on two: the first two, spread across the tile rather than crammed
  // into its left half.
  expect(origins(notesPath(four, 2)).map(([x]) => x)).toEqual([0, 50]);
});

it('caches per loop length, so a resize is not handed the old picture', () => {
  // The memo is keyed on the pattern object, which a resize does not replace.
  const four = clip({ bars: 4, lanes: [lane([0, 3840])] });
  const whole = notesPath(four, 4);
  const half = notesPath(four, 2);
  expect(whole).not.toBe(half);
  expect(notesPath(four, 4)).toBe(whole);
});

// ── What a clip can be handed over as ───────────────────────────────────────

it('a clip with notes is always MIDI', () => {
  // Notes are notes whether or not this build has a voice for them — the same
  // rule `DragRows` states for the Stems panel's MIDI handle.
  const format = clipFormat(clip({ lanes: [lane([0], 'snap')] }), new Set(), true);
  expect(format.midi).toBe(true);
  expect(format.audio).toBe(false);
});

it('a clip is audio only when something in it has a sample behind it', () => {
  const drums = clip({ lanes: [lane([0], 'kick')] });
  expect(clipFormat(drums, new Set<Lane>(['kick']), true).audio).toBe(true);
  expect(clipFormat(drums, new Set<Lane>(['snare']), true).audio).toBe(false);
});

it('an unread kit answers yes rather than hiding the audio handle', () => {
  // ⚠ `DragRows.audible` records why: the cost of being wrong this way is one
  // badge shown for a fraction of a second, and the cost of the other way is
  // the audio drag vanishing with nothing on screen saying why.
  expect(clipFormat(clip({ lanes: [lane([0])] }), new Set(), false).audio).toBe(true);
});

it('an empty clip offers neither format', () => {
  // A file of nothing is one a producer imports and has to work out was always
  // empty — the same reason the Stems panel skips an unwritten lane.
  expect(clipFormat(clip({ lanes: [lane([])] }), new Set(), false)).toEqual({
    midi: false,
    audio: false,
  });
  expect(clipFormat(null, new Set(), false)).toEqual({ midi: false, audio: false });
});
