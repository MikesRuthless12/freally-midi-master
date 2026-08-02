import { describe, expect, it } from 'vitest';

import type { Lane, Note } from '../../lib/ipc-types';
import {
  CAP_RADIUS,
  LANE_HEIGHT,
  line,
  reset,
  shift,
  stemId,
  stemNear,
  stemsFor,
  sweep,
  velocityToY,
  yToVelocity,
  type Track,
} from './velocity';

const note = (startTick: number, pitch: number, vel: number, modelVel?: number): Note => ({
  startTick,
  lenTicks: 240,
  pitch,
  vel,
  ...(modelVel === undefined ? {} : { modelVel }),
});

const track = (notes: Note[], lane: Lane = 'melody'): Track[] => [{ lane, notes }];

/** The lane's own x mapping in these tests: one pixel per tick, for legibility. */
const xOf = (tick: number) => tick;

const id = (n: Note, lane: Lane = 'melody') => stemId(lane, n);

describe('the value axis', () => {
  it('puts the quietest note at the bottom and the loudest at the top', () => {
    expect(velocityToY(1, LANE_HEIGHT)).toBeGreaterThan(velocityToY(127, LANE_HEIGHT));
  });

  it('round-trips a pointer height back to the velocity it asked for', () => {
    for (const vel of [1, 20, 64, 100, 127]) {
      expect(yToVelocity(velocityToY(vel, LANE_HEIGHT), LANE_HEIGHT)).toBe(vel);
    }
  });

  it('cannot be dragged past either end of MIDI', () => {
    expect(yToVelocity(-500, LANE_HEIGHT)).toBe(127);
    expect(yToVelocity(500, LANE_HEIGHT)).toBe(1);
  });

  it('keeps the loudest cap inside the canvas it is drawn on', () => {
    expect(velocityToY(127, LANE_HEIGHT)).toBeGreaterThanOrEqual(CAP_RADIUS);
    expect(velocityToY(1, LANE_HEIGHT)).toBeLessThanOrEqual(LANE_HEIGHT);
  });
});

describe('the sliders', () => {
  it('draws one per note, at that note’s place on the timeline', () => {
    const stems = stemsFor(track([note(0, 60, 100), note(480, 62, 80)]), xOf);
    expect(stems.map((s) => [s.x, s.note.vel])).toEqual([
      [0, 100],
      [480, 80],
    ]);
  });

  it('spreads notes sharing a tick so neither can hide the other', () => {
    // ⛔ The roadmap's own case: a kick and an 808 on beat 1 are two values.
    const stems = stemsFor(
      [
        { lane: 'kick', notes: [note(0, 36, 120)] },
        { lane: 'bass808', notes: [note(0, 24, 90)] },
      ],
      xOf,
    );
    expect(new Set(stems.map((s) => s.x)).size).toBe(2);
    // Two lanes, two sliders, and each writes back to its own.
    expect(stems.map((s) => s.lane)).toEqual(['bass808', 'kick']);
  });

  it('grabs the nearest cap, and nothing at all from far away', () => {
    const first = note(0, 60, 100);
    const stems = stemsFor(track([first, note(100, 62, 80)]), xOf);
    expect(stemNear(stems, 2)?.id).toBe(id(first));
    expect(stemNear(stems, 50)).toBeNull();
  });
});

describe('painting across the lane', () => {
  const notes = [note(0, 60, 100), note(100, 60, 100), note(200, 60, 100), note(900, 60, 100)];
  const stems = stemsFor(track(notes), xOf);

  it('levels every note the pointer passed, to the height it is at', () => {
    const values = sweep(stems, 0, 200, 64);
    expect(Object.values(values)).toEqual([64, 64, 64]);
    // The note the drag never reached is untouched.
    expect(values[id(notes[3])]).toBeUndefined();
  });

  it('writes a slope when the drag is on a slope, one segment at a time', () => {
    // Two moves at two heights: what the pointer passed at each height keeps it.
    const first = sweep(stems, 0, 100, 40);
    const second = sweep(stems, 100, 200, 120);
    expect(first[id(notes[0])]).toBe(40);
    expect(second[id(notes[2])]).toBe(120);
    expect(second[id(notes[0])]).toBeUndefined();
  });

  it('shift-drags a straight line between the two ends of the gesture', () => {
    const values = line(stems, { x: 0, vel: 20 }, { x: 200, vel: 120 });
    expect(values[id(notes[0])]).toBe(20);
    expect(values[id(notes[1])]).toBe(70);
    expect(values[id(notes[2])]).toBe(120);
  });
});

describe('dragging a selection', () => {
  const notes = [note(0, 60, 100), note(100, 62, 60)];
  const stems = stemsFor(track(notes), xOf);
  const all = new Set(notes.map((n) => id(n)));

  it('moves every selected note by the same amount, keeping the accents', () => {
    const values = shift(stems, all, -20);
    expect(values[id(notes[0])]).toBe(80);
    expect(values[id(notes[1])]).toBe(40);
  });

  it('stops the whole selection when one note reaches the ceiling', () => {
    // ⛔ Otherwise the quiet note keeps climbing after the loud one has stopped,
    // and the gesture that was meant to preserve the accent pattern erases it.
    const values = shift(stems, all, 50);
    expect(values[id(notes[0])]).toBe(127);
    expect(values[id(notes[1])]).toBe(87);
  });

  it('ignores notes that are not in the selection', () => {
    expect(shift(stems, new Set([id(notes[1])]), 10)).toEqual({ [id(notes[1])]: 70 });
  });
});

describe('resetting to what the model wrote', () => {
  it('puts back the pre-humanize velocity, not a flat default', () => {
    const notes = [note(0, 60, 127, 42), note(100, 62, 10, 90)];
    const stems = stemsFor(track(notes), xOf);
    expect(reset(stems, new Set(stems.map((s) => s.id)))).toEqual({
      [id(notes[0])]: 42,
      [id(notes[1])]: 90,
    });
  });

  it('leaves a note the producer drew alone, having no model opinion to restore', () => {
    const stems = stemsFor(track([note(0, 60, 100)]), xOf);
    expect(reset(stems, new Set(stems.map((s) => s.id)))).toEqual({});
  });
});
