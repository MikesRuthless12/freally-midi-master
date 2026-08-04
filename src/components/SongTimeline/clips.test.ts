/**
 * Clip editing (TASK-063B).
 *
 * The invariant every one of these protects is the same: **the sections tile
 * the song end to end, with no gap and no overlap, after every operation.** A
 * gap is silence the ruler does not draw; an overlap is two sections claiming
 * one bar. Both survive a screenshot and both make the export disagree with what
 * the producer arranged.
 */

import { describe, expect, it } from 'vitest';

import type { Pattern, Song } from '../../lib/ipc-types';
import {
  cloneSection,
  copyClips,
  deleteClips,
  isSelected,
  partsInUse,
  pasteClips,
  resizeSection,
  totalBars,
} from './clips';

function pattern(id: string): Pattern {
  return {
    id,
    part: 'drums',
    artistId: 'trap',
    seed: '1',
    bars: 4,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 6,
    scale: 'natural_minor',
    lanes: [],
    ppq: 960,
  };
}

/** intro(4) verse(16) hook(8) — the shape `_defaults` actually produces. */
function song(): Song {
  return {
    id: 's',
    artistId: 'trap',
    seed: '1',
    bpm: 140,
    keyRoot: 6,
    scale: 'natural_minor',
    timeSigNum: 4,
    timeSigDen: 4,
    ppq: 960,
    patterns: {
      'p-drums': pattern('p-drums'),
      'p-melody': pattern('p-melody'),
    },
    sections: [
      {
        type: 'intro',
        startBar: 0,
        bars: 4,
        patterns: { melody: { patternId: 'p-melody' } },
        dropOutBeats: 0,
        decay: false,
        markers: [],
      },
      {
        type: 'verse',
        startBar: 4,
        bars: 16,
        patterns: { drums: { patternId: 'p-drums' }, melody: { patternId: 'p-melody' } },
        dropOutBeats: 2,
        decay: false,
        markers: [],
      },
      {
        type: 'hook',
        startBar: 20,
        bars: 8,
        patterns: { drums: { patternId: 'p-drums' } },
        dropOutBeats: 0,
        decay: false,
        markers: [],
      },
    ],
  };
}

/** The invariant, asserted after every operation below. */
function expectTiled(result: Song) {
  let expected = 0;
  for (const section of result.sections) {
    expect(section.startBar).toBe(expected);
    expect(section.bars).toBeGreaterThanOrEqual(1);
    expected += section.bars;
  }
  expect(totalBars(result)).toBe(expected);
}

describe('resize', () => {
  it('moves everything after it rather than leaving a gap', () => {
    const result = resizeSection(song(), 0, 8);
    expect(result.sections[0].bars).toBe(8);
    expect(result.sections[1].startBar).toBe(8);
    expect(result.sections[2].startBar).toBe(24);
    expectTiled(result);
  });

  it('will not shrink a section out of existence', () => {
    // A zero-bar section is a hole in the song, not a short one — `arrange.rs`
    // refuses to build one and this refuses to drag one into being.
    for (const bars of [0, -4]) {
      const result = resizeSection(song(), 1, bars);
      expect(result.sections[1].bars).toBe(1);
      expectTiled(result);
    }
  });

  it('leaves the song untouched when nothing changed', () => {
    const before = song();
    expect(resizeSection(before, 1, 16)).toBe(before);
    expect(resizeSection(before, 99, 8)).toBe(before);
  });
});

describe('clone', () => {
  it('places the copy directly after the original and shifts the rest', () => {
    const result = cloneSection(song(), 2);
    expect(result.sections).toHaveLength(4);
    expect(result.sections[3].type).toBe('hook');
    expect(result.sections[3].startBar).toBe(28);
    expectTiled(result);
  });

  it('shares the pattern rather than copying the notes', () => {
    // ⛔ Two sections playing one clip is what a clone *is*, and it is what
    // `arrange.rs` already produces for two verses. Copying the pattern would
    // double the document to express something the format says in one word —
    // and it would materialise a clip that was never edited, which the
    // 2026-07-31 decision rules out.
    const result = cloneSection(song(), 1);
    expect(result.sections[2].patterns.drums?.patternId).toBe('p-drums');
    expect(Object.keys(result.patterns)).toHaveLength(2);
  });

  it('gives the copy its own patterns object', () => {
    // Shared by reference, a later delete on the clone would empty the original.
    const result = deleteClips(cloneSection(song(), 1), [{ sectionIndex: 2, part: 'drums' }]);
    expect(result.sections[1].patterns.drums).toBeDefined();
    expect(result.sections[2].patterns.drums).toBeUndefined();
  });
});

describe('delete', () => {
  it('deleting a clip leaves its section standing', () => {
    // "Delete a MIDI part and re-add it" — removing the drums from a verse is
    // not removing the verse, and one gesture must not do the other.
    const result = deleteClips(song(), [{ sectionIndex: 1, part: 'drums' }]);
    expect(result.sections).toHaveLength(3);
    expect(result.sections[1].patterns.drums).toBeUndefined();
    expect(result.sections[1].patterns.melody).toBeDefined();
    expectTiled(result);
  });
});

describe('copy and paste', () => {
  it('round-trips a clip onto another section', () => {
    const clipboard = copyClips(song(), [{ sectionIndex: 1, part: 'drums' }]);
    expect(clipboard).not.toBeNull();
    const result = pasteClips(song(), clipboard!, 0);
    expect(result.sections[0].patterns.drums?.patternId).toBe('p-drums');
    expectTiled(result);
  });

  it('refuses to paste a clip whose pattern the song no longer holds', () => {
    // ⛔ A dangling ref draws as an empty row and exports as silence, naming a
    // pattern the producer deleted three actions ago. The store is the
    // authority on what exists.
    const before = song();
    const result = pasteClips(before, { clips: [{ part: 'bass', patternId: 'gone' }] }, 0);
    expect(result).toBe(before);
    expect(result.sections[0].patterns.bass).toBeUndefined();
  });

  it('copying nothing yields no clipboard rather than an empty one', () => {
    // An empty clipboard would make the next paste a silent no-op that looks
    // like the paste is broken.
    expect(copyClips(song(), [])).toBeNull();
    expect(copyClips(song(), [{ sectionIndex: 0, part: 'bass' }])).toBeNull();
  });
});

describe('selection and rows', () => {
  it('matches a clip by section and part together', () => {
    const selection = [{ sectionIndex: 1, part: 'drums' as const }];
    expect(isSelected(selection, { sectionIndex: 1, part: 'drums' })).toBe(true);
    // Same row, different section — the bug a part-only comparison would have.
    expect(isSelected(selection, { sectionIndex: 2, part: 'drums' })).toBe(false);
    expect(isSelected(selection, { sectionIndex: 1, part: 'melody' })).toBe(false);
  });

  it('draws only the rows something actually plays on', () => {
    expect(partsInUse(song())).toEqual(['drums', 'melody']);
  });
});
