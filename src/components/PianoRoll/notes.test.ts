import { describe, expect, it } from 'vitest';

import {
  addNote,
  deleteNotes,
  duplicateNotes,
  laneOf,
  MAX_PITCH,
  moveNotes,
  noteAt,
  noteId,
  notesInBox,
  notesOf,
  patternTicks,
  resizeNotes,
  selectionLength,
  snapTick,
  snapTicks,
  SNAP_VALUES,
  withNotes,
} from './notes';
import type { Lane, Note, Pattern } from '../../lib/ipc-types';

const PPQ = 960;

function note(startTick: number, pitch: number, lenTicks = 240, vel = 100): Note {
  return { startTick, lenTicks, pitch, vel };
}

function pattern(lanes: { lane: Lane; notes: Note[] }[], bars = 1): Pattern {
  return {
    id: 't',
    part: 'melody',
    artistId: 't',
    seed: '1',
    bars,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'natural_minor',
    lanes,
    ppq: PPQ,
  };
}

const ids = (...notes: Note[]) => new Set(notes.map(noteId));

describe('snapping', () => {
  it('divides a whole note exactly, triplets included', () => {
    // ⛔ The reason PPQ is 960. A division that came out fractional would put a
    // tick of drift into every triplet, which accumulates into an audible flam.
    for (const snap of SNAP_VALUES) {
      const step = snapTicks(snap, PPQ);
      if (step === null) continue;
      expect(Number.isInteger(step), `${snap} is ${step}`).toBe(true);
    }
  });

  it('puts a 16th triplet at 160 ticks, three to the eighth', () => {
    expect(snapTicks('1/16T', PPQ)).toBe(160);
    expect(snapTicks('1/8', PPQ)).toBe(480);
    expect(160 * 3).toBe(480);
  });

  it('rounds to the nearest step and never below zero', () => {
    expect(snapTick(239, '1/16', PPQ)).toBe(240);
    expect(snapTick(241, '1/16', PPQ)).toBe(240);
    expect(snapTick(-50, '1/16', PPQ)).toBe(0);
  });

  it('leaves the tick alone when snapping is off', () => {
    // The whole point of `off`: an edit against a humanized performance must be
    // able to sit where the performance sits.
    expect(snapTick(217, 'off', PPQ)).toBe(217);
  });
});

describe('lane addressing', () => {
  it('names each melodic part its own lane', () => {
    expect(laneOf('melody')).toBe('melody');
    expect(laneOf('bass')).toBe('bass');
  });

  it('reports an absent lane as empty rather than throwing', () => {
    expect(notesOf(pattern([]), 'melody')).toEqual([]);
  });
});

describe('withNotes', () => {
  it('shares every lane it did not touch', () => {
    // ⛔ This is the claim `state/history.ts` rests unlimited undo on. If an
    // edit rebuilt the untouched lanes, a hundred-step history would hold a
    // hundred copies of them instead of a hundred pointers to one.
    const chords = { lane: 'chords' as Lane, notes: [note(0, 60)] };
    const before = pattern([{ lane: 'melody', notes: [note(0, 64)] }, chords]);

    const after = withNotes(before, 'melody', [note(240, 64)]);

    expect(after.lanes.find((track) => track.lane === 'chords')).toBe(chords);
  });

  it('appends a lane the pattern does not have yet', () => {
    // Drawing the first note on an empty clip goes through here.
    const after = withNotes(pattern([]), 'melody', [note(0, 64)]);
    expect(notesOf(after, 'melody')).toHaveLength(1);
  });

  it('keeps notes sorted by tick then pitch', () => {
    const after = withNotes(pattern([]), 'melody', [note(480, 60), note(0, 67), note(0, 62)]);
    expect(notesOf(after, 'melody').map((n) => [n.startTick, n.pitch])).toEqual([
      [0, 62],
      [0, 67],
      [480, 60],
    ]);
  });
});

describe('adding and deleting', () => {
  it('adds a note', () => {
    const after = addNote(pattern([]), 'melody', note(240, 64));
    expect(notesOf(after, 'melody')).toEqual([note(240, 64)]);
  });

  it('trims a note that would outlive the clip', () => {
    // ⛔ `engine/src/midi.rs` refuses to export a note past the pattern end, so
    // one drawn long would look held on screen and arrive shortened in the DAW.
    const ticks = patternTicks(pattern([]));
    const after = addNote(pattern([]), 'melody', note(ticks - 100, 64, 5000));
    expect(notesOf(after, 'melody')[0].lenTicks).toBe(100);
  });

  it('deletes only the selection', () => {
    const keep = note(0, 60);
    const drop = note(240, 62);
    const after = deleteNotes(
      pattern([{ lane: 'melody', notes: [keep, drop] }]),
      'melody',
      ids(drop),
    );
    expect(notesOf(after, 'melody')).toEqual([keep]);
  });

  it('returns the same pattern when nothing is selected', () => {
    const before = pattern([{ lane: 'melody', notes: [note(0, 60)] }]);
    expect(deleteNotes(before, 'melody', new Set())).toBe(before);
  });
});

describe('moving', () => {
  it('moves the selection by the delta', () => {
    const n = note(240, 60);
    const after = moveNotes(
      pattern([{ lane: 'melody', notes: [n] }]),
      'melody',
      ids(n),
      240,
      2,
    );
    expect(notesOf(after, 'melody')[0]).toMatchObject({ startTick: 480, pitch: 62 });
  });

  it('holds a chord together at the pitch floor', () => {
    // ⛔ The reason the delta is clamped for the selection rather than per note.
    // Clamping each note on its own would let the upper voices fall while the
    // lowest stuck, silently re-voicing the chord.
    const low = note(0, 1);
    const high = note(0, 8);
    const before = pattern([{ lane: 'melody', notes: [low, high] }]);

    const after = moveNotes(before, 'melody', ids(low, high), 0, -5);

    const pitches = notesOf(after, 'melody').map((n) => n.pitch);
    expect(pitches).toEqual([0, 7]);
    expect(pitches[1] - pitches[0]).toBe(7);
  });

  it('refuses to move past the clip start', () => {
    const n = note(0, 60);
    const before = pattern([{ lane: 'melody', notes: [n] }]);
    expect(moveNotes(before, 'melody', ids(n), -240, 0)).toBe(before);
  });

  it('keeps one note when a move lands two on the same tick and pitch', () => {
    // Two notes at one tick and pitch are one audible note and two MIDI events,
    // the second of which cuts the first short.
    const stay = note(0, 60);
    const moved = note(240, 60);
    const before = pattern([{ lane: 'melody', notes: [stay, moved] }]);

    const after = moveNotes(before, 'melody', ids(moved), -240, 0);

    expect(notesOf(after, 'melody')).toHaveLength(1);
  });

  it('never sends a pitch above MIDI', () => {
    const n = note(0, MAX_PITCH);
    const before = pattern([{ lane: 'melody', notes: [n] }]);
    expect(moveNotes(before, 'melody', ids(n), 0, 5)).toBe(before);
  });
});

describe('resizing', () => {
  it('moves the start and keeps the end on the left edge', () => {
    const n = note(480, 60, 480);
    const after = resizeNotes(
      pattern([{ lane: 'melody', notes: [n] }]),
      'melody',
      ids(n),
      'start',
      240,
      60,
    );
    const out = notesOf(after, 'melody')[0];
    expect(out.startTick).toBe(720);
    expect(out.startTick + out.lenTicks).toBe(960);
  });

  it('moves the end and keeps the start on the right edge', () => {
    const n = note(0, 60, 240);
    const after = resizeNotes(
      pattern([{ lane: 'melody', notes: [n] }]),
      'melody',
      ids(n),
      'end',
      240,
      60,
    );
    expect(notesOf(after, 'melody')[0]).toMatchObject({ startTick: 0, lenTicks: 480 });
  });

  it('will not shrink a note below the floor', () => {
    const n = note(0, 60, 240);
    const after = resizeNotes(
      pattern([{ lane: 'melody', notes: [n] }]),
      'melody',
      ids(n),
      'end',
      -1000,
      60,
    );
    expect(notesOf(after, 'melody')[0].lenTicks).toBe(60);
  });

  it('resizes a multi-selection by the shortest note, so none inverts', () => {
    const short = note(0, 60, 120);
    const long = note(0, 64, 960);
    const before = pattern([{ lane: 'melody', notes: [short, long] }]);

    const after = resizeNotes(before, 'melody', ids(short, long), 'end', -900, 60);

    const lengths = notesOf(after, 'melody').map((n) => n.lenTicks);
    expect(Math.min(...lengths)).toBe(60);
    expect(lengths).toEqual([60, 900]);
  });
});

describe('duplicating', () => {
  it('copies the selection one selection-length later', () => {
    const n = note(0, 60, 480);
    const before = pattern([{ lane: 'melody', notes: [n] }]);

    const result = duplicateNotes(
      before,
      'melody',
      ids(n),
      selectionLength(before, 'melody', ids(n)),
    );

    expect(notesOf(result.pattern, 'melody').map((x) => x.startTick)).toEqual([0, 480]);
  });

  it('reports the copies so Duplicate can leave them selected', () => {
    // ⛔ What makes Shift+D twice produce three spaced copies rather than three
    // stacked on the second.
    const n = note(0, 60, 480);
    const before = pattern([{ lane: 'melody', notes: [n] }]);

    const result = duplicateNotes(before, 'melody', ids(n), 480);

    expect(result.ids).toEqual(['480:60']);
  });

  it('measures a selection from its earliest start to its latest end', () => {
    const a = note(0, 60, 240);
    const b = note(480, 64, 240);
    const before = pattern([{ lane: 'melody', notes: [a, b] }]);
    expect(selectionLength(before, 'melody', ids(a, b))).toBe(720);
  });

  it('is a no-op with nothing selected', () => {
    const before = pattern([{ lane: 'melody', notes: [note(0, 60)] }]);
    const result = duplicateNotes(before, 'melody', new Set(), 480);
    expect(result.pattern).toBe(before);
    expect(result.ids).toEqual([]);
  });

  it('drops a copy that would fall past the end of the clip', () => {
    // Clamping instead would pile every copy onto the last tick, which is not
    // a duplicate of anything.
    const ticks = patternTicks(pattern([]));
    const n = note(ticks - 240, 60, 240);
    const before = pattern([{ lane: 'melody', notes: [n] }]);

    const result = duplicateNotes(before, 'melody', ids(n), 480);

    expect(result.ids).toEqual([]);
    expect(notesOf(result.pattern, 'melody')).toHaveLength(1);
  });
});

describe('hit testing', () => {
  it('catches a note the marquee crosses without enclosing', () => {
    // ⛔ Touched, not enclosed. A held note wider than the screen would be
    // unselectable at any useful zoom if containment were required.
    const held = note(0, 60, 3840);
    const before = pattern([{ lane: 'melody', notes: [held] }]);

    const hit = notesInBox(before, 'melody', {
      fromTick: 1000,
      toTick: 1100,
      fromPitch: 59,
      toPitch: 61,
    });

    expect(hit).toEqual([noteId(held)]);
  });

  it('leaves out a note outside the pitch range', () => {
    const n = note(0, 72);
    const before = pattern([{ lane: 'melody', notes: [n] }]);
    expect(
      notesInBox(before, 'melody', { fromTick: 0, toTick: 960, fromPitch: 59, toPitch: 61 }),
    ).toEqual([]);
  });

  it('finds the note under a point and nothing past its end', () => {
    const n = note(240, 60, 240);
    const before = pattern([{ lane: 'melody', notes: [n] }]);
    expect(noteAt(before, 'melody', 300, 60)).toMatchObject({ startTick: 240 });
    expect(noteAt(before, 'melody', 480, 60)).toBeNull();
    expect(noteAt(before, 'melody', 300, 61)).toBeNull();
  });
});
