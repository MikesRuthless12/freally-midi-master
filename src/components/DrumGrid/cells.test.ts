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
  clearCells,
  copyCells,
  pasteCells,
  columnOf,
  reassignLane,
  unusedLanes,
  addFill,
  PITCHED_LANES,
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
      // ── TASK-043A ──────────────────────────────────────────────────────
      'subKick',
      'ghostSnare',
      'pedalHat',
      'rideBell',
      'tomHigh',
      'tomLow',
      'perc2',
      'clave',
      'conga',
      'bongo',
      'timbale',
      'triangle',
      'riser',
      'impact',
      'reverse',
      'subLow',
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

  describe('clearCells — the right-drag sweep (TASK-056 #3)', () => {
    it('wipes every cell the drag crossed, in one new pattern', () => {
      const source = clip([
        { startTick: 0 },
        { startTick: 240 },
        { startTick: 480 },
        { startTick: 720 },
      ]);
      const swept = clearCells(source, [
        { lane: 'closedHat', column: 1 },
        { lane: 'closedHat', column: 2 },
      ]);
      expect(hats(swept).map((n) => n.startTick)).toEqual([0, 720]);
    });

    it('crosses lanes, because a drag does', () => {
      const source: Pattern = {
        ...clip([{ startTick: 0 }]),
        lanes: [
          ...clip([{ startTick: 0 }]).lanes,
          {
            lane: 'kick',
            notes: [
              {
                startTick: 0,
                lenTicks: 240,
                pitch: 36,
                vel: 100,
                modelVel: null,
                slideToPitch: null,
                articulation: null,
              },
            ],
          },
        ],
      };
      const swept = clearCells(source, [
        { lane: 'closedHat', column: 0 },
        { lane: 'kick', column: 0 },
      ]);
      expect(hats(swept)).toEqual([]);
      expect(swept.lanes.find((l) => l.lane === 'kick')?.notes).toEqual([]);
    });

    it('takes a humanized hit with the cell it is drawn in', () => {
      // ⛔ The defect this module has already been bitten by twice: half the
      // hits in a generated pattern sit early of the grid, and an exact 16th
      // span misses exactly those. A sweep that leaves them behind reads as the
      // drag having skipped cells at random.
      const early = clip([{ startTick: TICKS_PER_16TH * 4 - 33 }]);
      expect(hats(clearCells(early, [{ lane: 'closedHat', column: 4 }]))).toEqual([]);
      // And the visually empty cell to its left still must not touch it.
      expect(hats(clearCells(early, [{ lane: 'closedHat', column: 3 }]))).toHaveLength(1);
    });

    it('returns the very same pattern when the sweep hit nothing', () => {
      // A right-drag across empty cells must not push an undo step.
      const source = clip([{ startTick: 0 }]);
      expect(
        clearCells(source, [
          { lane: 'closedHat', column: 5 },
          { lane: 'closedHat', column: 6 },
        ]),
      ).toBe(source);
      expect(clearCells(source, [])).toBe(source);
    });

    it('takes a whole roll with the cell, the way one click does', () => {
      const rolled = tuplet(clip([]), 'closedHat', 2, 3);
      expect(hats(rolled)).toHaveLength(3);
      expect(hats(clearCells(rolled, [{ lane: 'closedHat', column: 2 }]))).toEqual([]);
    });
  });

  describe('copy, paste and clone (TASK-056 #4)', () => {
    const at = (lane: Lane, column: number) => ({ lane, column });

    it('puts the copied block down where it was asked for', () => {
      const source = clip([{ startTick: 0 }, { startTick: 240 }]);
      const lifted = copyCells(source, [at('closedHat', 0), at('closedHat', 1)]);
      expect(lifted).not.toBeNull();
      expect(lifted!.columns).toBe(2);

      const pasted = pasteCells(source, lifted!, 8);
      expect(
        hats(pasted)
          .map((n) => n.startTick)
          .sort((a, b) => a - b),
      ).toEqual([0, 240, 1920, 2160]);
    });

    it('keeps a roll intact rather than quantising it onto the grid', () => {
      // ⛔ The reason the clip stores ticks and not cell indices. A triplet is
      // three notes 80 ticks apart *inside* one 16th; rebuilding it on the grid
      // would hand back three 16ths, which is a different figure.
      const rolled = tuplet(clip([]), 'closedHat', 0, 3);
      const lifted = copyCells(rolled, [at('closedHat', 0)]);
      const pasted = pasteCells(rolled, lifted!, 4);
      const landed = hats(pasted)
        .map((n) => n.startTick)
        .filter((tick) => tick >= 960 - 40 && tick < 1200)
        .sort((a, b) => a - b);
      expect(landed).toEqual([960, 1040, 1120]);
    });

    it('carries a humanized hit at the offset it actually sat at', () => {
      const early = clip([{ startTick: TICKS_PER_16TH * 4 - 33 }]);
      const lifted = copyCells(early, [at('closedHat', 4)]);
      const pasted = pasteCells(early, lifted!, 8);
      expect(
        hats(pasted)
          .map((n) => n.startTick)
          .sort((a, b) => a - b),
      ).toEqual([TICKS_PER_16TH * 4 - 33, TICKS_PER_16TH * 8 - 33]);
    });

    it('clears the destination first rather than doubling into a flam', () => {
      // The same rule `cloneBar` keeps, and for the same reason.
      const source = clip([
        { startTick: 0, vel: 120 },
        { startTick: 960, vel: 60 },
      ]);
      const lifted = copyCells(source, [at('closedHat', 0)]);
      const pasted = pasteCells(source, lifted!, 4);
      const landed = hats(pasted).filter((n) => n.startTick >= 920 && n.startTick < 1160);
      expect(landed).toHaveLength(1);
      expect(landed[0].vel).toBe(120);
    });

    it('leaves every note in its own lane', () => {
      // ⛔ Sliding the block onto whichever row had focus would turn a kick
      // pattern into a crash pattern. Moving a lane is `reassignLane`'s job.
      const source: Pattern = {
        ...clip([{ startTick: 0 }]),
        lanes: [
          ...clip([{ startTick: 0 }]).lanes,
          {
            lane: 'kick',
            notes: [
              {
                startTick: 0,
                lenTicks: 240,
                pitch: 36,
                vel: 100,
                modelVel: null,
                slideToPitch: null,
                articulation: null,
              },
            ],
          },
        ],
      };
      const lifted = copyCells(source, [at('closedHat', 0), at('kick', 0)]);
      const pasted = pasteCells(source, lifted!, 4);
      expect(
        pasted.lanes.find((l) => l.lane === 'kick')?.notes.map((n) => n.startTick),
      ).toEqual([0, 960]);
      expect(
        hats(pasted)
          .map((n) => n.startTick)
          .sort((a, b) => a - b),
      ).toEqual([0, 960]);
    });

    it('drops what would land past the end instead of piling it on the last cell', () => {
      const source = clip([{ startTick: 0 }, { startTick: 240 }]);
      const lifted = copyCells(source, [at('closedHat', 0), at('closedHat', 1)]);
      // The clip is two bars, so column 31 is its last: the block's second note
      // has nowhere to go.
      const pasted = pasteCells(source, lifted!, 31);
      const tail = hats(pasted).filter((n) => n.startTick >= 7000);
      expect(tail.map((n) => n.startTick)).toEqual([31 * TICKS_PER_16TH]);
    });

    it('has nothing to copy when the selected cells are empty', () => {
      // ⚠ A clipboard holding an empty block would let Ctrl+V wipe a region
      // while looking like it pasted.
      expect(copyCells(clip([{ startTick: 0 }]), [at('closedHat', 9)])).toBeNull();
      expect(copyCells(clip([{ startTick: 0 }]), [])).toBeNull();
    });

    it('clones the block immediately after itself', () => {
      // What Ctrl+D does: lift columns 0–1 and put them down at 2–3.
      const source = clip([{ startTick: 0 }, { startTick: 240 }]);
      const picked = [at('closedHat', 0), at('closedHat', 1)];
      const lifted = copyCells(source, picked)!;
      const cloned = pasteCells(source, lifted, 0 + lifted.columns);
      expect(
        hats(cloned)
          .map((n) => n.startTick)
          .sort((a, b) => a - b),
      ).toEqual([0, 240, 480, 720]);
    });
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

describe('reassignLane (TASK-043A)', () => {
  const base = pattern([
    { lane: 'kick', notes: [{ startTick: 0, lenTicks: 120, pitch: 36, vel: 100 }] },
    { lane: 'snare', notes: [{ startTick: 960, lenTicks: 120, pitch: 38, vel: 100 }] },
  ]);

  it('offers only the lanes the kit is not already using', () => {
    const free = unusedLanes(base);
    expect(free).not.toContain('kick');
    expect(free).not.toContain('snare');
    expect(free).toContain('cowbell');
    // ⛔ Two slots on one lane is a row the producer edits and cannot hear, so
    // the picker never offers a taken lane rather than validating afterwards.
    expect(new Set(free).size).toBe(free.length);
  });

  it('never offers a pitched lane as a drum slot', () => {
    // ⛔⛔ **The 808s got through the melodic-parts argument.** `sub` and
    // `subLow` are rows of this grid, so `LANE_ORDER` holds them — but they are
    // pitched, and `reassignLane` moves notes unchanged on purpose. Picking a
    // perc row over to "808" therefore exported unpitched drum hits down the
    // pitched channel as bass notes at whatever pitch they carried.
    for (const lane of PITCHED_LANES) {
      expect(LANE_ORDER).toContain(lane);
      expect(unusedLanes(base)).not.toContain(lane);
    }
  });

  it('never offers the four melodic parts as a drum slot', () => {
    // ⛔ **This passed vacuously once.**  filtered the melodic
    // lanes out explicitly, which read as a safeguard over a list that has
    // never contained one — so the assertion could not fail whatever the code
    // did. The filter is gone; what actually guarantees it is ,
    // and this now asserts that instead.
    for (const part of ['melody', 'counter', 'bass', 'chords']) {
      expect(LANE_ORDER).not.toContain(part);
      expect(unusedLanes(base)).not.toContain(part);
    }
  });

  it('moves the notes and leaves them exactly as they were', () => {
    const moved = reassignLane(base, 'snare', 'cowbell');
    expect(moved.lanes.map((l) => l.lane).sort()).toEqual(['cowbell', 'kick']);
    // ⚠ The notes are untouched — re-pitching them to the new lane's GM note is
    // the exporter's job, and doing it here would bake a drum map into the clip.
    expect(moved.lanes.find((l) => l.lane === 'cowbell')?.notes).toEqual(
      base.lanes.find((l) => l.lane === 'snare')?.notes,
    );
  });

  it('refuses a lane already in use rather than merging two slots', () => {
    // Merging would silently destroy one of them, which is the failure the
    // picker's "unused only" rule exists to make unreachable.
    expect(reassignLane(base, 'snare', 'kick')).toBe(base);
    expect(reassignLane(base, 'snare', 'snare')).toBe(base);
    expect(reassignLane(base, 'conga', 'cowbell')).toBe(base);
  });
});

describe('addFill (TASK-043H)', () => {
  // A one-bar clip at 4/4: 3840 ticks, sixteen 16ths of 240.
  const hats = pattern([
    {
      lane: 'closedHat',
      notes: Array.from({ length: 16 }, (_, i) => ({
        startTick: i * 240,
        lenTicks: 120,
        pitch: 42,
        vel: 90,
      })),
    },
  ]);

  it('writes the fill into the last beat and leaves the rest of the bar alone', () => {
    // ⛔ **The last beat, and only it.** `rolls::hat_fills` puts its window at
    // `bar_ticks - ticks_per_beat`; a hand-added fill that landed anywhere else
    // would be a figure the generator would never write, so a producer who then
    // pressed Generate would watch theirs move.
    const filled = addFill(hats, 'closedHat');
    const notes = filled.lanes.find((l) => l.lane === 'closedHat')!.notes;

    const before = notes.filter((n) => n.startTick < 2880);
    expect(before).toEqual(hats.lanes[0].notes.slice(0, 12));

    const inFill = notes.filter((n) => n.startTick >= 2880);
    // Four 16ths at two hits each — a 32nd stream, 120 ticks apart.
    expect(inFill.map((n) => n.startTick)).toEqual([
      2880, 3000, 3120, 3240, 3360, 3480, 3600, 3720,
    ]);
    // ⛔ Nothing past the end of the bar: the filter that drops those is what
    // keeps a fill from writing notes the clip has no room to play.
    expect(notes.every((n) => n.startTick < 3840)).toBe(true);
  });

  it('ramps once across the whole figure rather than per cell', () => {
    // ⛔ A fill is one gesture. Four cells each ramping 45→100 would read as
    // four little crescendos — busy, not a hand-over.
    const notes = addFill(hats, 'closedHat').lanes[0].notes.filter((n) => n.startTick >= 2880);
    const vels = notes.map((n) => n.vel);
    expect(vels).toEqual([...vels].sort((a, b) => a - b));
    expect(vels[0]).toBeLessThan(vels[vels.length - 1]);
    expect(vels[vels.length - 1]).toBe(127);
  });

  it('takes the pitch the lane already plays, so a fill on the hats is hats', () => {
    const notes = addFill(hats, 'closedHat').lanes[0].notes;
    expect(new Set(notes.map((n) => n.pitch))).toEqual(new Set([42]));
  });

  it('takes the last beat, not four sixteenths, when a beat is not a quarter', () => {
    // ⛔⛔ **`FILL_SIXTEENTHS` was the constant 4 and a beat is not always
    // four sixteenths.** In 6/8 it is two — `PPQ * 4 / 8` — so the fill cleared
    // and rewrote *two beats* of hats, destroying a beat the producer never
    // asked to lose and writing a figure twice as long as any the engine would
    // produce. The comment promising this matched `rolls::hat_fills` was false
    // for every meter but x/4.
    const eight = { ...hats, timeSigNum: 6, timeSigDen: 8 };
    const filled = addFill(eight, 'closedHat');
    const notes = filled.lanes.find((l) => l.lane === 'closedHat')!.notes;

    // One bar of 6/8 is 6 × (960 * 4 / 8) = 2880 ticks; the last beat starts at
    // 2400, which is column 10 of twelve.
    const inFill = notes.filter((n) => n.startTick >= 2400);
    expect(inFill.map((n) => n.startTick)).toEqual([2400, 2520, 2640, 2760]);
    // ⛔ And the beat before it is untouched — this is the half the bug ate.
    expect(notes.filter((n) => n.startTick >= 1920 && n.startTick < 2400)).toHaveLength(2);
  });

  it('still writes a fill on a lane that is empty in the bar so far', () => {
    // No note before the window means no template to copy a pitch from. The
    // producer still asked for a fill, so it has to arrive — silently doing
    // nothing is the readout-that-lies shape: the button reports success.
    const empty = pattern([{ lane: 'openHat', notes: [] }]);
    const notes = addFill(empty, 'openHat').lanes.find((l) => l.lane === 'openHat')!.notes;
    expect(notes.length).toBe(8);
  });
});
