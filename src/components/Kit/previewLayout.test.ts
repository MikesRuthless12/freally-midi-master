import { describe, expect, it } from 'vitest';

import {
  allNotes,
  previewBars,
  songClips,
  patternTicks,
  MAX_ROWS,
  MIN_NOTE_WIDTH,
  MIN_CLIP_WIDTH,
  PREVIEW_HEIGHT,
  PREVIEW_LABEL_HEIGHT,
  PREVIEW_WIDTH,
} from './previewLayout';
import { toBase64 } from './dragPreview';
import type { Note, Part, Pattern, Song } from '../../lib/ipc-types';

/**
 * A song of the given sections, each naming the parts it plays.
 *
 * ⚠ Only the fields `songClips` reads are meaningful — `partsInUse` walks
 * `sections[].patterns` and `totalBars` sums `sections[].bars`. The rest is
 * there because `Song` is a real type and a partial one would let the fixture
 * drift from what the app actually hands in.
 */
function songOf(sections: { startBar: number; bars: number; parts: Part[] }[]): Song {
  return {
    id: 'fixture',
    artistId: 'fixture',
    seed: '1',
    bpm: 140,
    keyRoot: 0,
    scale: 'natural_minor',
    timeSigNum: 4,
    timeSigDen: 4,
    ppq: 960,
    patterns: {},
    sections: sections.map(({ startBar, bars, parts }) => ({
      type: 'verse' as const,
      startBar,
      bars,
      dropOutBeats: 0,
      decay: false,
      patterns: Object.fromEntries(
        parts.map((part) => [part, { patternId: 'a' }]),
      ) as Song['sections'][number]['patterns'],
    })),
  };
}

function note(startTick: number, pitch: number, extra: Partial<Note> = {}): Note {
  return {
    startTick,
    lenTicks: 240,
    pitch,
    vel: 100,
    modelVel: null,
    slideToPitch: null,
    articulation: null,
    ...extra,
  };
}

function pattern(extra: Partial<Pattern> = {}): Pattern {
  return {
    id: 't',
    part: 'drums',
    artistId: 'trap',
    seed: '7',
    songSeed: '7',
    bars: 4,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 1,
    scale: 'natural_minor',
    lanes: [{ lane: 'kick', notes: [note(0, 36)] }],
    ppq: 960,
    mood: null,
    loopRegion: null,
    clipRegion: null,
    ...extra,
  };
}

describe('the drag image', () => {
  it('measures a clip with the app-wide definition, not one of its own', () => {
    // ⛔ `patternTicks` is *the* definition — `SongTimeline/sketch.ts` records
    // that "a private copy here would have been the ninth", and the drag image
    // very nearly made it the tenth. This asserts it is re-exported rather than
    // reimplemented, so a meter change lands in one place.
    expect(patternTicks(pattern())).toBe(960 * 4 * 4);
    expect(patternTicks(pattern({ timeSigNum: 3, bars: 2 }))).toBe(960 * 3 * 2);
    expect(patternTicks(pattern({ timeSigDen: 8, bars: 1 }))).toBe(480 * 4);
    // ⚠ **A meter of 0 is not guarded here, deliberately.** The shared
    // definition does not guard it, adding a guard only in this copy is exactly
    // what makes a tenth definition, and the plugin refuses such a pattern
    // outright (`check_patterns`) — so the drag is rejected before anyone sees
    // a picture of it. The worst case is a cosmetic one on a clip that cannot
    // be dragged anyway.
    expect(patternTicks(pattern({ bars: 0 }))).toBeGreaterThan(0);
  });

  it('draws the arrangement as clips at their true bars, not a density graph', () => {
    // ⛔⛔ **Mike, 2026-08-06:** *"i want the song arrangement being dragged in to
    // actually show the midi clips either together back to back or stacked."*
    // The old drawing was one card of density bars whatever the drop contained
    // — the *"purple graph"* he objected to.
    const clips = songClips(
      songOf([
        { startBar: 0, bars: 4, parts: ['drums', 'melody'] },
        { startBar: 4, bars: 8, parts: ['drums'] },
      ]),
    );

    expect(clips).toHaveLength(3);
    // The intro's two clips share a bar and differ by row.
    expect(clips[0].x).toBe(0);
    expect(clips[1].x).toBe(0);
    expect(clips[1].y).toBeGreaterThan(clips[0].y);
    // The verse starts a third of the way in and is twice as long.
    expect(clips[2].x).toBeCloseTo(PREVIEW_WIDTH / 3);
    expect(clips[2].width).toBeGreaterThan(clips[0].width);
    for (const clip of clips) {
      expect(clip.y).toBeGreaterThanOrEqual(PREVIEW_LABEL_HEIGHT);
      expect(clip.y + clip.height).toBeLessThanOrEqual(PREVIEW_HEIGHT);
    }
  });

  it('stacks every clip at bar 1 when the modifier is held, and keeps its length', () => {
    // ⚠ They overlap on purpose. That is what stacking *does*, and a picture
    // that tidied them into a row would describe a third layout no modifier
    // produces.
    const song = songOf([
      { startBar: 0, bars: 4, parts: ['drums'] },
      { startBar: 4, bars: 8, parts: ['drums'] },
    ]);
    const stacked = songClips(song, true);
    const inLine = songClips(song, false);

    expect(stacked.every((clip) => clip.x === 0)).toBe(true);
    // Length is untouched — only the position moves.
    expect(stacked.map((c) => c.width)).toEqual(inLine.map((c) => c.width));
  });

  it('gives a one-bar section a rectangle rather than a hairline', () => {
    // ⛔⛔ **The song has to be LONG enough to drive the raw width under the
    // floor, or this test passes with the floor deleted.** At 16 total bars a
    // one-bar clip is already 260/16 − 1 = 15.25px wide and the clamp never
    // engages — the assertion held on the arithmetic alone and proved nothing
    // about `MIN_CLIP_WIDTH`. The floor only starts doing work past ~87 total
    // bars, which is an ordinary record rather than an edge case: 160 bars is
    // about four minutes at 4/4.
    const [clip] = songClips(
      songOf([
        { startBar: 0, bars: 1, parts: ['drums'] },
        { startBar: 1, bars: 159, parts: ['drums'] },
      ]),
    );

    // Raw would be 260/160 − 1 = 0.625px — a hairline, or nothing at all once
    // the canvas rounds it. ⚠ Asserted as equality, not `>=`: the floor is what
    // produced this number, so a regression that removed it fails here rather
    // than passing on a wider clip that happened to satisfy a bound.
    expect(clip.width).toBe(MIN_CLIP_WIDTH);
  });

  it('draws nothing rather than claiming a record that is not there', () => {
    // ⛔ An empty song must not paint a full-width block. Three ways in:
    // No sections at all.
    expect(songClips(songOf([]))).toEqual([]);
    // Sections, but none of them plays anything.
    expect(songClips(songOf([{ startBar: 0, bars: 4, parts: [] }]))).toEqual([]);
    // A section of zero bars, so the song has no length to scale against.
    expect(songClips(songOf([{ startBar: 0, bars: 0, parts: ['drums'] }]))).toEqual([]);
  });

  it('spreads the notes over the pitches the clip actually uses', () => {
    // ⛔ Not over all 128: a four-note bassline drawn on a full keyboard is
    // four marks in a smear of empty space.
    const bars = previewBars([note(0, 60), note(960, 72)], 3840);
    expect(bars).toHaveLength(2);
    // The higher note is the top row, the lower one the bottom.
    expect(bars[1].y).toBeLessThan(bars[0].y);
    expect(bars[0].y + bars[0].height).toBeLessThanOrEqual(PREVIEW_HEIGHT);
    expect(bars[1].y).toBeGreaterThanOrEqual(PREVIEW_LABEL_HEIGHT);
  });

  it('draws a one-pitch clip on one row rather than dividing by zero', () => {
    // A kick lane is this shape most of the time.
    const bars = previewBars([note(0, 36), note(960, 36)], 3840);
    expect(bars).toHaveLength(2);
    for (const bar of bars) {
      expect(Number.isFinite(bar.y)).toBe(true);
      expect(bar.y).toBe(PREVIEW_LABEL_HEIGHT);
    }
  });

  it('never draws a note too thin to see', () => {
    const [bar] = previewBars([note(0, 36, { lenTicks: 1 })], 3840);
    expect(bar.width).toBeGreaterThanOrEqual(MIN_NOTE_WIDTH);
  });

  it('keeps a wide pitch range inside the picture', () => {
    const notes = Array.from({ length: 40 }, (_, i) => note(i * 96, 30 + i * 2));
    const bars = previewBars(notes, 3840);
    for (const bar of bars) {
      expect(bar.y).toBeGreaterThanOrEqual(PREVIEW_LABEL_HEIGHT);
      expect(bar.y + bar.height).toBeLessThanOrEqual(PREVIEW_HEIGHT + 1);
      expect(bar.x).toBeLessThanOrEqual(PREVIEW_WIDTH);
    }
    // However many pitches there are, the rows are capped.
    const rows = new Set(bars.map((bar) => bar.y));
    expect(rows.size).toBeLessThanOrEqual(MAX_ROWS);
  });

  it('has nothing to draw for an empty clip rather than dividing by nothing', () => {
    expect(previewBars([], 3840)).toEqual([]);
    expect(previewBars([note(0, 36)], 0)).toEqual([]);
  });

  it('reads every lane, because a drum clip is eight of them', () => {
    const drums = pattern({
      lanes: [
        { lane: 'kick', notes: [note(0, 36)] },
        { lane: 'snare', notes: [note(960, 38), note(2880, 38)] },
      ],
    });
    expect(allNotes([drums])).toHaveLength(3);
  });

  it('encodes a whole picture without blowing the stack', () => {
    // ⛔ `String.fromCharCode(...bytes)` on this many pixels throws
    // `RangeError: Maximum call stack size exceeded` — the argument list is the
    // limit, and a 260x92 image is 95,680 bytes.
    const bytes = new Uint8Array(PREVIEW_WIDTH * PREVIEW_HEIGHT * 4).fill(0x41);
    const encoded = toBase64(bytes);
    expect(encoded.length).toBeGreaterThan(100_000);
    // And it round-trips, which is what the Rust decoder will do to it.
    expect(atob(encoded).length).toBe(bytes.length);
  });
});
