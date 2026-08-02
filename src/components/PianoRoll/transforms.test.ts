import { describe, expect, it } from 'vitest';

import type { Lane, Note, Pattern } from '../../lib/ipc-types';
import { noteId, notesOf, type NoteId } from './notes';
import {
  humanize,
  invert,
  legato,
  quantize,
  reselect,
  reverse,
  stretch,
  transposeToScale,
} from './transforms';

const PPQ = 960;
const LANE: Lane = 'melody';

const note = (startTick: number, pitch: number, lenTicks = PPQ / 4, vel = 100): Note => ({
  startTick,
  lenTicks,
  pitch,
  vel,
});

function clip(notes: Note[], bars = 1): Pattern {
  return {
    id: 'x',
    part: 'melody',
    artistId: 'a',
    seed: '1',
    bars,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'major',
    ppq: PPQ,
    lanes: [{ lane: LANE, notes }],
  };
}

const all = (notes: Note[]): ReadonlySet<NoteId> => new Set(notes.map(noteId));
const out = (pattern: Pattern) =>
  notesOf(pattern, LANE).map((n) => [n.startTick, n.pitch, n.lenTicks] as const);

describe('invert', () => {
  it('lifts the lowest voice an octave, leaving the rest of the chord alone', () => {
    const triad = [note(0, 60), note(0, 64), note(0, 67)];
    expect(out(invert(clip(triad), LANE, all(triad)))).toEqual([
      [0, 64, 240],
      [0, 67, 240],
      [0, 72, 240],
    ]);
  });

  it('walks through the inversions when pressed again', () => {
    const triad = [note(0, 60), note(0, 64), note(0, 67)];
    const once = invert(clip(triad), LANE, all(triad));
    const twice = invert(once, LANE, all(notesOf(once, LANE)));
    expect(out(twice)).toEqual([
      [0, 67, 240],
      [0, 72, 240],
      [0, 76, 240],
    ]);
  });

  it('drops the highest voice instead when asked downward', () => {
    const triad = [note(0, 60), note(0, 64), note(0, 67)];
    expect(out(invert(clip(triad), LANE, all(triad), 'down'))).toEqual([
      [0, 55, 240],
      [0, 60, 240],
      [0, 64, 240],
    ]);
  });

  it('refuses rather than clamping when the octave would leave MIDI', () => {
    const high = [note(0, 120)];
    const pattern = clip(high);
    expect(invert(pattern, LANE, all(high))).toBe(pattern);
  });
});

describe('reverse', () => {
  it('mirrors the selection about its own span', () => {
    const notes = [note(0, 60, 480), note(480, 62, 240), note(720, 64, 240)];
    expect(out(reverse(clip(notes), LANE, all(notes)))).toEqual([
      [0, 64, 240],
      [240, 62, 240],
      [480, 60, 480],
    ]);
  });

  it('leaves the notes it was not given where they were', () => {
    const notes = [note(0, 60, 240), note(240, 62, 240), note(1920, 64, 240)];
    const only = new Set([noteId(notes[0]), noteId(notes[1])]);
    const after = out(reverse(clip(notes, 2), LANE, only));
    expect(after).toContainEqual([1920, 64, 240]);
  });
});

describe('stretch', () => {
  it('doubles the offsets and the durations, anchored at the first note', () => {
    const notes = [note(480, 60, 240), note(720, 62, 240)];
    expect(out(stretch(clip(notes, 2), LANE, all(notes), 2))).toEqual([
      [480, 60, 480],
      [960, 62, 480],
    ]);
  });

  it('halves them just as exactly', () => {
    const notes = [note(0, 60, 480), note(960, 62, 480)];
    expect(out(stretch(clip(notes), LANE, all(notes), 0.5))).toEqual([
      [0, 60, 240],
      [480, 62, 240],
    ]);
  });

  it('removes a note pushed past the clip rather than piling it on the last tick', () => {
    // ⛔ Two notes clamped to the same tick become one, and the producer is
    // never told which one they lost.
    const notes = [note(0, 60), note(1920, 62), note(2880, 64)];
    const after = out(stretch(clip(notes), LANE, all(notes), 2));
    expect(after).toEqual([[0, 60, 480]]);
  });
});

describe('legato', () => {
  it('extends each note to the next onset', () => {
    const notes = [note(0, 60, 120), note(480, 62, 120), note(960, 64, 120)];
    expect(out(legato(clip(notes), LANE, all(notes)))).toEqual([
      [0, 60, 480],
      [480, 62, 480],
      [960, 64, 120],
    ]);
  });

  it('keeps a chord’s voices the same length as each other', () => {
    // ⛔ "The next note in the list" would leave a triad at three lengths, one
    // of them zero — the notes on one tick are one event, not three in a row.
    const notes = [note(0, 60, 120), note(0, 64, 120), note(0, 67, 120), note(480, 69, 120)];
    expect(out(legato(clip(notes), LANE, all(notes)))).toEqual([
      [0, 60, 480],
      [0, 64, 480],
      [0, 67, 480],
      [480, 69, 120],
    ]);
  });
});

describe('quantize', () => {
  const grid = PPQ / 4;

  it('snaps hard at full strength', () => {
    const notes = [note(37, 60), note(263, 62)];
    expect(out(quantize(clip(notes), LANE, all(notes), grid, 1)).map((n) => n[0])).toEqual([
      0, 240,
    ]);
  });

  it('moves a note half of the way at half strength', () => {
    const notes = [note(40, 60)];
    expect(out(quantize(clip(notes), LANE, all(notes), grid, 0.5))[0][0]).toBe(20);
  });

  it('changes nothing at all at zero, rather than snapping quietly', () => {
    const notes = [note(37, 60)];
    const pattern = clip(notes);
    expect(quantize(pattern, LANE, all(notes), grid, 0)).toBe(pattern);
  });
});

describe('humanize', () => {
  it('moves every note by a bounded amount, and never out of MIDI', () => {
    // A fixed source, so this asserts values rather than "something changed":
    // each note draws once for its timing and once for its velocity, so `1, 0`
    // is "as late as allowed, as quiet as allowed".
    const rolls = [1, 0];
    let index = 0;
    const random = () => rolls[index++ % rolls.length];

    const notes = [note(480, 60, 240, 100), note(720, 62, 240, 100)];
    const after = notesOf(
      humanize(clip(notes, 2), LANE, all(notes), { ticks: 20, velocity: 10 }, random),
      LANE,
    );
    expect(after.map((n) => [n.startTick, n.vel])).toEqual([
      [500, 90],
      [740, 90],
    ]);
  });

  it('pins an early note at the start of the clip instead of wrapping', () => {
    const notes = [note(0, 60)];
    const after = notesOf(
      humanize(clip(notes), LANE, all(notes), { ticks: 40, velocity: 0 }, () => 0),
      LANE,
    );
    expect(after[0].startTick).toBe(0);
  });
});

describe('transposeToScale', () => {
  // C major.
  const classes = new Set([0, 2, 4, 5, 7, 9, 11]);

  it('leaves notes that are already in the key exactly where they are', () => {
    const notes = [note(0, 60), note(240, 64)];
    const pattern = clip(notes);
    expect(out(transposeToScale(pattern, LANE, all(notes), classes))).toEqual(out(pattern));
  });

  it('folds an out-of-key note to the nearest in-key pitch, tying upward', () => {
    // C♯ is one from both C and D; F♯ is one from both F and G.
    const notes = [note(0, 61), note(240, 66)];
    expect(
      out(transposeToScale(clip(notes), LANE, all(notes), classes)).map((n) => n[1]),
    ).toEqual([62, 67]);
  });

  it('does nothing when the key is not known yet rather than flattening the clip', () => {
    const notes = [note(0, 61)];
    const pattern = clip(notes);
    expect(transposeToScale(pattern, LANE, all(notes), new Set())).toBe(pattern);
  });
});

describe('the selection a transform hands back', () => {
  it('follows the notes it just renamed, so transforms can be chained', () => {
    // ⛔ A `NoteId` is a start tick and a pitch, and every transform here moves
    // one or both. Without this the second press in "invert, then legato" would
    // act on an empty selection and look like the menu had stopped working.
    const triad = [note(0, 60), note(0, 64), note(0, 67)];
    const before = clip(triad);
    const after = invert(before, LANE, all(triad));
    expect(reselect(before, after, LANE, all(triad)).sort()).toEqual(
      notesOf(after, LANE).map(noteId).sort(),
    );
  });

  it('does not sweep up a note the transform never touched', () => {
    const notes = [note(0, 60), note(1920, 62)];
    const only = new Set([noteId(notes[0])]);
    const before = clip(notes, 2);
    const after = invert(before, LANE, only);
    expect(reselect(before, after, LANE, only)).toEqual([noteId({ ...notes[0], pitch: 72 })]);
  });
});
