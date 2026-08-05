import { describe, expect, it } from 'vitest';

import {
  allNotes,
  previewBars,
  sectionBars,
  patternTicks,
  MAX_ROWS,
  MIN_NOTE_WIDTH,
  PREVIEW_HEIGHT,
  PREVIEW_LABEL_HEIGHT,
  PREVIEW_WIDTH,
} from './previewLayout';
import { toBase64 } from './dragPreview';
import type { Note, Pattern } from '../../lib/ipc-types';

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

  it('draws the arrangement as its sections, not as every note in it', () => {
    // Every note of a three-minute record in 260 pixels is a solid block, which
    // says nothing about which record it is.
    const bars = sectionBars([0, 0.5, 1]);
    expect(bars).toHaveLength(3);
    expect(bars[2].height).toBeGreaterThan(bars[1].height);
    expect(bars[1].height).toBeGreaterThan(bars[0].height);
    for (const bar of bars) {
      expect(bar.y).toBeGreaterThanOrEqual(PREVIEW_LABEL_HEIGHT);
      expect(bar.y + bar.height).toBeLessThanOrEqual(PREVIEW_HEIGHT);
      expect(bar.alpha).toBeGreaterThan(0);
      expect(bar.alpha).toBeLessThanOrEqual(1);
    }
    // An empty section list has nothing to draw rather than dividing by zero.
    expect(sectionBars([])).toEqual([]);
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
