import { describe, expect, it } from 'vitest';

import {
  columnDensity,
  LANE_ORDER,
  TICKS_PER_16TH,
  toCells,
  toggleHit,
  tuplet,
  cloneBar,
  clearCell,
  columnOf,
} from './cells';
import type { Lane, Note, Pattern } from '../../lib/ipc-types';

function pattern(lanes: { lane: Lane; notes: Note[] }[], bars = 1): Pattern {
  return {
    id: 't',
    part: 'drums',
    artistId: 't',
    seed: '1',
    songSeed: '1',
    bars,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'natural_minor',
    lanes,
    ppq: 960,
  };
}

const note = (startTick: number, vel = 100): Note => ({
  startTick,
  lenTicks: 120,
  pitch: 36,
  vel,
});

describe('toCells', () => {
  it('gives a bar sixteen columns', () => {
    const [row] = toCells(pattern([{ lane: 'kick', notes: [note(0)] }]));
    expect(row.cells).toHaveLength(16);
    expect(toCells(pattern([{ lane: 'kick', notes: [note(0)] }], 4))[0].cells).toHaveLength(64);
  });

  it('puts a hit in the column it belongs to', () => {
    const [row] = toCells(
      pattern([{ lane: 'kick', notes: [note(0), note(TICKS_PER_16TH * 6)] }]),
    );
    const on = row.cells.map((c, i) => (c.hits > 0 ? i : -1)).filter((i) => i >= 0);
    expect(on).toEqual([0, 6]);
  });

  it('keeps a humanized note on its own beat', () => {
    // The engine writes off the grid on purpose. A plain floor would drag a
    // note nudged early into the previous cell, so the grid would show a beat
    // the file does not contain — and the drift would only ever be backwards,
    // which reads as a rhythm rather than as a bug. 33 ticks is ~14 ms at
    // 140 BPM: the largest jitter any shipped model authors.
    const early = TICKS_PER_16TH * 4 - 33;
    const late = TICKS_PER_16TH * 8 + 33;
    const [row] = toCells(pattern([{ lane: 'kick', notes: [note(early), note(late)] }]));
    const on = row.cells.map((c, i) => (c.hits > 0 ? i : -1)).filter((i) => i >= 0);
    expect(on).toEqual([4, 8]);
  });

  it('counts a 32nd roll as two hits in one cell rather than a run of 16ths', () => {
    // The grid is 16ths and a roll is finer, so the two notes of a 32nd have
    // to stack. Rounding to the nearest column would push the second into the
    // next cell and the roll would look exactly like ordinary 16ths — the
    // generator's most audible flourish, invisible.
    const [row] = toCells(
      pattern([{ lane: 'closedHat', notes: [note(0), note(TICKS_PER_16TH / 2)] }]),
    );
    expect(row.cells[0].hits).toBe(2);
    expect(row.cells[1].hits).toBe(0);
  });

  it('takes the loudest velocity in a cell', () => {
    const [row] = toCells(
      pattern([{ lane: 'kick', notes: [note(0, 40), note(TICKS_PER_16TH / 2, 120)] }]),
    );
    expect(row.cells[0].velocity).toBe(120);
  });

  it('drops a note that lands past the end rather than wrapping it to the start', () => {
    // Wrapping would put a hit on the downbeat that nothing generated — the
    // most convincing wrong thing this component could draw.
    const [row] = toCells(pattern([{ lane: 'kick', notes: [note(0), note(960 * 4 + 480)] }]));
    expect(row.cells.filter((c) => c.hits > 0)).toHaveLength(1);
  });

  it('shows only the lanes the pattern actually has, in kit order', () => {
    const rows = toCells(
      pattern([
        { lane: 'kick', notes: [note(0)] },
        { lane: 'closedHat', notes: [note(0)] },
      ]),
    );
    expect(rows.map((r) => r.lane)).toEqual(['closedHat', 'kick']);
  });

  it('orders every lane the engine can produce', () => {
    // A lane missing from LANE_ORDER is a lane that silently never draws.
    const everyLane: Lane[] = [
      'kick',
      'snare',
      'offSnare',
      'clap',
      'closedHat',
      'openHat',
      'ride',
      'crash',
      'tom',
      'rim',
      'snap',
      'perc',
      'shaker',
      'tambourine',
      'cowbell',
      'woodblock',
      'sub',
    ];
    expect([...LANE_ORDER].sort()).toEqual([...everyLane].sort());
  });
});

describe('columnDensity', () => {
  it('lights the columns the notes are actually in', () => {
    // What makes the ripple ignite cells where the beat is, rather than
    // sweeping a uniform bar across an empty grid (FR-017).
    const density = columnDensity(
      pattern([{ lane: 'kick', notes: [note(0), note(960 * 2)] }]),
      4,
    );
    expect(density).toHaveLength(4);
    expect(density[0]).toBeGreaterThan(0);
    expect(density[2]).toBeGreaterThan(0);
    expect(density[1]).toBe(0);
    expect(density[3]).toBe(0);
  });

  it('normalises, so a sparse pattern lights up as much as a dense one', () => {
    // The shape carries the information, not the absolute count. Without this
    // a boom-bap pattern would barely glow next to a drill one.
    const sparse = columnDensity(pattern([{ lane: 'kick', notes: [note(0)] }]), 4);
    const dense = columnDensity(
      pattern([{ lane: 'closedHat', notes: [note(0), note(10), note(20), note(30)] }]),
      4,
    );
    expect(Math.max(...sparse)).toBe(1);
    expect(Math.max(...dense)).toBe(1);
  });

  it('stays inside its buckets whatever the note positions', () => {
    // A note exactly on the final tick must not index one past the end — that
    // is an undefined the draw loop would silently treat as an unlit column.
    const density = columnDensity(
      pattern([{ lane: 'kick', notes: [note(0), note(960 * 4 - 1), note(960 * 8)] }]),
      8,
    );
    expect(density).toHaveLength(8);
    expect(density.every((d) => Number.isFinite(d) && d >= 0 && d <= 1)).toBe(true);
  });

  it('returns all zeroes for a pattern with no notes rather than dividing by none', () => {
    const density = columnDensity(pattern([{ lane: 'kick', notes: [] }]), 4);
    expect(density).toEqual([0, 0, 0, 0]);
  });
});

describe('editing the grid (TASK-131G)', () => {
  const clip = (notes: { startTick: number; vel?: number; pitch?: number }[]): Pattern => ({
    id: 't',
    part: 'drums',
    artistId: 'trap',
    seed: '1',
    songSeed: '1',
    bars: 2,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'natural_minor',
    lanes: [
      {
        lane: 'closedHat',
        notes: notes.map((n) => ({
          startTick: n.startTick,
          lenTicks: 240,
          pitch: n.pitch ?? 42,
          vel: n.vel ?? 100,
          modelVel: null,
          slideToPitch: null,
          articulation: null,
        })),
      },
    ],
    ppq: 960,
    mood: null,
    loopRegion: null,
    clipRegion: null,
  });

  const hats = (p: Pattern) => p.lanes.find((l) => l.lane === 'closedHat')?.notes ?? [];

  it('places a hit in an empty cell and clears one that has any', () => {
    const empty = clip([]);
    const placed = toggleHit(empty, 'closedHat', 3);
    expect(hats(placed).map((n) => n.startTick)).toEqual([720]);

    // ⚠ Clearing removes *every* hit in the cell, roll included — leaving two of
    // three would be a state the grid cannot show, since the cell would still
    // read as occupied.
    const roll = clip([{ startTick: 720 }, { startTick: 800 }, { startTick: 880 }]);
    expect(hats(toggleHit(roll, 'closedHat', 3))).toEqual([]);
  });

  it('adds the lane when the pattern has no track for it yet', () => {
    // Otherwise the first hat placed on a pattern that only has a kick goes
    // nowhere and the click reads as broken.
    const placed = toggleHit(clip([]), 'kick', 0);
    expect(placed.lanes.find((l) => l.lane === 'kick')?.notes).toHaveLength(1);
  });

  it('splits a cell into a triplet at real sub-16th ticks', () => {
    // ⛔ **The control Mike named: Ctrl+3.** 240 ticks / 3 is 80, which is not a
    // 16th boundary — this is the whole reason the edits work on ticks and not
    // on cells.
    const split = tuplet(clip([{ startTick: 480, pitch: 46, vel: 77 }]), 'closedHat', 2, 3);
    expect(
      hats(split)
        .map((n) => n.startTick)
        .sort((a, b) => a - b),
    ).toEqual([480, 560, 640]);
    // The cell's own sound carries over, so a triplet of *open* hats stays open
    // hats rather than becoming three of the default.
    expect(hats(split).every((n) => n.pitch === 46 && n.vel === 77)).toBe(true);
  });

  it('makes a quintuplet too, and never a zero-length note', () => {
    const five = tuplet(clip([]), 'closedHat', 0, 5);
    expect(hats(five)).toHaveLength(5);
    // ⚠ A zero-length note is what `pattern_to_smf` cannot pair a note-off
    // against — the collision class this repo has already been bitten by.
    expect(hats(five).every((n) => n.lenTicks >= 1)).toBe(true);
  });

  it('still draws a tuplet as a multi-hit cell rather than losing it', () => {
    // ⚠ The claim that the 16th bucketing had to be replaced before tuplets
    // could exist was wrong: `toCells` counts hits per cell, so three notes
    // inside one 16th draw as a three-hit cell — the same way the 32nd rolls the
    // generator already writes have always drawn.
    const rows = toCells(tuplet(clip([]), 'closedHat', 1, 3));
    const cells = rows.find((r) => r.lane === 'closedHat')!.cells;
    expect(cells[1].hits).toBe(3);
  });

  it('clones a bar over another and does not double what is already there', () => {
    // ⚠ Merging would double every hit the two bars share, which reads as the
    // clone having worked and sounds like a flam.
    const source = clip([{ startTick: 0 }, { startTick: 480 }, { startTick: 3840 }]);
    const cloned = cloneBar(source, 'closedHat', 0, 1);
    const inBarTwo = hats(cloned)
      .filter((n) => n.startTick >= 3840)
      .map((n) => n.startTick)
      .sort((a, b) => a - b);
    expect(inBarTwo).toEqual([3840, 4320]);
  });

  it('clears a cell and returns the same object when there is nothing there', () => {
    // `editPattern` reference-compares and drops a no-op, so returning the
    // pattern itself is what makes Delete on an empty cell free.
    const source = clip([{ startTick: 0 }]);
    const cleared = clearCell(source, 'closedHat', 0);
    expect(hats(cleared)).toEqual([]);
    // The row survives: `toCells` only draws lanes the pattern has, so dropping
    // the track would make it vanish under the producer's cursor mid-edit.
    expect(cleared.lanes.some((l) => l.lane === 'closedHat')).toBe(true);
    expect(clearCell(cleared, 'closedHat', 0)).toBe(cleared);
  });
});

describe('editing a humanized pattern (the case the first tests missed)', () => {
  // ⛔ The unit tests for TASK-131G all used notes exactly on the grid — the one
  // shape the engine never produces. `humanize` jitters every hit, so roughly
  // half of them sit a few ticks EARLY, and `toCells` buckets those forward with
  // EARLY_TOLERANCE while the edit functions used an exact 16th span. The cell a
  // producer saw lit and the cell an edit targeted disagreed.
  const jittered = (startTick: number): Pattern => ({
    id: 't',
    part: 'drums',
    artistId: 'trap',
    seed: '1',
    songSeed: '1',
    bars: 2,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'natural_minor',
    lanes: [
      {
        lane: 'closedHat',
        notes: [
          {
            startTick,
            lenTicks: 240,
            pitch: 42,
            vel: 100,
            modelVel: null,
            slideToPitch: null,
            articulation: null,
          },
        ],
      },
    ],
    ppq: 960,
    mood: null,
    loopRegion: null,
    clipRegion: null,
  });

  const hats = (p: Pattern) => p.lanes.find((l) => l.lane === 'closedHat')?.notes ?? [];

  it('clears the cell the hit is DRAWN in, not the one its raw tick falls in', () => {
    // A hat written at 960 but nudged to 953 draws in column 4.
    const early = jittered(953);
    const drawn = toCells(early).find((r) => r.lane === 'closedHat')!.cells;
    expect(drawn[4].hits).toBe(1);
    expect(drawn[3].hits).toBe(0);

    // Clicking the lit cell must clear it — it used to append a second hit and
    // play an audible flam.
    expect(hats(toggleHit(early, 'closedHat', 4))).toEqual([]);
    // Delete on it used to be a silent no-op.
    expect(hats(clearCell(early, 'closedHat', 4))).toEqual([]);
    // And the visually empty cell to its left must not touch it.
    expect(hats(clearCell(early, 'closedHat', 3))).toHaveLength(1);
  });

  it('does not double a humanized downbeat when a bar is cloned over it', () => {
    // Bar 2's kick written at 3840 but jittered to 3834 sits just BELOW the bar
    // line, so an exact boundary left it in place while the copy landed on the
    // line — two hits a few ticks apart, which is a flam from the one gesture
    // whose contract is that the destination is cleared first.
    const source = jittered(3834);
    source.lanes[0].notes.push({
      startTick: 0,
      lenTicks: 240,
      pitch: 42,
      vel: 100,
      modelVel: null,
      slideToPitch: null,
      articulation: null,
    });

    const cloned = cloneBar(source, 'closedHat', 0, 1);
    const inBarTwo = hats(cloned).filter((n) => n.startTick >= 3800);
    expect(inBarTwo).toHaveLength(1);
    expect(inBarTwo[0].startTick).toBe(3840);
  });

  it('leaves a sextuplet spill clearable from the cell it is drawn in', () => {
    // A tuplet keeps its TRUE subdivision — a 16th triplet is 80 ticks, and
    // squeezing it to fit one column would make it not a triplet. Sizes of six
    // and up therefore put their last note in the next column; what matters is
    // that clicking that column finds it.
    const six = tuplet(jittered(960), 'closedHat', 4, 6);
    const strayColumn = columnOf(
      hats(six)
        .map((n) => n.startTick)
        .sort((a, b) => b - a)[0],
    );
    expect(strayColumn).toBe(5);
    // Clicking cell 5 used to append yet another hit instead of clearing it.
    const after = clearCell(six, 'closedHat', strayColumn);
    expect(hats(after).length).toBeLessThan(hats(six).length);
  });
});
