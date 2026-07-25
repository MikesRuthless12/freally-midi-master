import { describe, expect, it } from 'vitest';

import { columnDensity, LANE_ORDER, TICKS_PER_16TH, toCells } from './cells';
import type { Lane, Note, Pattern } from '../../lib/ipc-types';

function pattern(lanes: { lane: Lane; notes: Note[] }[], bars = 1): Pattern {
  return {
    id: 't',
    part: 'drums',
    artistId: 't',
    seed: '1',
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
      'clap',
      'closedHat',
      'openHat',
      'rim',
      'snap',
      'perc',
      'bass808',
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
