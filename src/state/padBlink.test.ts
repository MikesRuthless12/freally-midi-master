import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { BLINK_MS, subscribeToPadBlink } from './padBlink';
import { useSession } from './session';
import type { Pattern } from '../lib/ipc-types';

/**
 * Lighting a pad as its lane fires (2026-08-11).
 *
 * Mike: *"the drum pads should blink with a color as the drum parts play or the
 * melodic parts play for the drum pads or the melody/chords/basslines/counter
 * melody."*
 *
 * ⚠ **Asserted on the DOM attribute, because there is no state to assert on.**
 * That is the design rather than an inconvenience: `playhead` moves 30 times a
 * second, and routing this through React would re-render every pad and every kit
 * row at that rate. `padBlink.ts` carries the reasoning.
 */

/** One bar of 4/4 at 960 PPQ, so a beat is a clean quarter of the pattern. */
const DRUMS: Pattern = {
  id: 'trap-drums',
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
  ppq: 960,
  lanes: [
    // Beat 1 and beat 3.
    {
      lane: 'kick',
      notes: [
        { startTick: 0, lenTicks: 240, pitch: 36, vel: 110 },
        { startTick: 1920, lenTicks: 240, pitch: 36, vel: 110 },
      ],
    },
    // Beat 2 only.
    { lane: 'snare', notes: [{ startTick: 960, lenTicks: 240, pitch: 38, vel: 100 }] },
  ],
};

const MELODY: Pattern = {
  ...DRUMS,
  id: 'trap-melody',
  part: 'melody',
  lanes: [{ lane: 'melody', notes: [{ startTick: 960, lenTicks: 480, pitch: 72, vel: 90 }] }],
};

let stop: () => void = () => {};

function pad(lane: string): HTMLElement {
  const node = document.createElement('div');
  node.className = 'pad';
  node.dataset.lane = lane;
  document.body.append(node);
  return node;
}

function kitRow(lane: string): HTMLElement {
  const node = document.createElement('li');
  node.className = 'kit-lane';
  node.dataset.lane = lane;
  document.body.append(node);
  return node;
}

/** Move the playhead the way the poll does, and let the subscription see it. */
function to(fraction: number) {
  useSession.setState({ playhead: fraction });
}

beforeEach(() => {
  vi.useFakeTimers();
  document.body.innerHTML = '';
  useSession.setState({
    patterns: { drums: DRUMS, melody: MELODY },
    playhead: 0,
    playing: true,
    mutedLanes: [],
    soloedLanes: [],
  });
});

afterEach(() => {
  stop();
  stop = () => {};
  vi.useRealTimers();
});

describe('a pad lights as its lane fires', () => {
  it('lights only the lanes the playhead just crossed', () => {
    const kick = pad('kick');
    const snare = pad('snare');
    stop = subscribeToPadBlink();

    // ⚠ Step to beat 2 first, so the window under test starts *after* the
    // downbeat rather than on it — the kick's second hit is at beat 3 (0.5) and
    // is what must stay dark.
    to(0.2);
    for (const node of [kick, snare]) delete node.dataset.lit;

    // Past beat 2 (0.25) but not beat 3 (0.5).
    to(0.3);

    expect(snare.dataset.lit).toBe('true');
    expect(kick.dataset.lit).toBeUndefined();
  });

  it('lights the downbeat on the very first pass', () => {
    // ⛔⛔ **`crossed` was half-open at the low end**, so with the playhead at
    // rest on 0 the first tick asked `0 > 0` and a kick on beat 1 did not light —
    // on the first bar only, because from the second pass the wrap branch's
    // `at <= now` catches it. One bar in every playback missing its downbeat.
    const kick = pad('kick');
    stop = subscribeToPadBlink();

    to(0.02);

    expect(kick.dataset.lit).toBe('true');
  });

  it('lights nothing when the producer scrubs the playhead backwards', () => {
    // ⛔⛔ **A scrub is not a loop point.** `seek` moves the playhead backwards
    // with the transport still rolling, and the wrap window is two ranges — so
    // without `seekNonce` a drag of the ruler from bar 4 back to bar 1 flashed
    // every pad with a hit in *either* range at once. Nothing between sounded.
    const kick = pad('kick');
    const snare = pad('snare');
    stop = subscribeToPadBlink();

    to(0.9);
    for (const node of [kick, snare]) delete node.dataset.lit;

    // What `seek` does: the position moves back and the nonce moves on.
    useSession.setState((state) => ({ playhead: 0.1, seekNonce: state.seekNonce + 1 }));

    expect(kick.dataset.lit).toBeUndefined();
    expect(snare.dataset.lit).toBeUndefined();
  });

  it('lights a melodic lane in the kit list, which has no pad', () => {
    // ⛔ The half of the ask that has nowhere else to show: melody, chords, bass
    // and counter are not on the pad grid at all.
    const melody = kitRow('melody');
    stop = subscribeToPadBlink();

    to(0.3);

    expect(melody.dataset.lit).toBe('true');
  });

  it('goes dark again on its own', () => {
    const snare = pad('snare');
    stop = subscribeToPadBlink();
    to(0.3);
    expect(snare.dataset.lit).toBe('true');

    vi.advanceTimersByTime(BLINK_MS + 1);
    expect(snare.dataset.lit).toBeUndefined();
  });

  it('catches the hits between the last frame and the loop point', () => {
    // ⛔⛔ **The wrap is a jump backwards, not a gap.** Treating it as an
    // ordinary step means every hit between the last frame of the bar and the
    // end of it is skipped — on every repeat, which is most of the time the
    // producer is looking at this.
    const kick = pad('kick');
    stop = subscribeToPadBlink();
    // ⚠ Walked past beat 3 first and left to go dark, so what lights below can
    // only have come from the wrap. Stepping straight to 0.9 crosses the kick on
    // the way and the assertion would pass on the wrong hit.
    to(0.9);
    vi.advanceTimersByTime(BLINK_MS + 1);
    expect(kick.dataset.lit).toBeUndefined();

    // Round the loop: 0.9 -> 0.02 crosses the kick sitting on tick 0.
    to(0.02);
    expect(kick.dataset.lit).toBe('true');
  });

  it('leaves a muted lane dark', () => {
    // ⚠ A pad lighting for something the producer cannot hear is the
    // readout-that-lies failure the pads' own dot exists to prevent.
    useSession.setState({ mutedLanes: ['snare'] });
    const snare = pad('snare');
    stop = subscribeToPadBlink();

    to(0.3);

    expect(snare.dataset.lit).toBeUndefined();
  });

  it('leaves every unsoloed lane dark while something is soloed', () => {
    useSession.setState({ soloedLanes: ['kick'] });
    const snare = pad('snare');
    const kick = pad('kick');
    stop = subscribeToPadBlink();

    to(0.3);
    expect(snare.dataset.lit).toBeUndefined();

    to(0.6);
    expect(kick.dataset.lit).toBe('true');
  });

  it('does not fire everything on beat one when the transport stops', () => {
    // ⛔ `stop` sets the playhead to 0, which is a *movement*. Without the
    // guard, pressing Stop flashes every lane that has a note on beat 1.
    const kick = pad('kick');
    stop = subscribeToPadBlink();
    to(0.9);
    // Cleared first, so what is asserted below is the *stop* and not the beat-3
    // kick this walk already crossed.
    vi.advanceTimersByTime(BLINK_MS + 1);

    useSession.setState({ playing: false, playhead: 0 });

    expect(kick.dataset.lit).toBeUndefined();
  });

  it('stops writing once it is unsubscribed', () => {
    const snare = pad('snare');
    stop = subscribeToPadBlink();
    stop();
    stop = () => {};

    to(0.3);

    expect(snare.dataset.lit).toBeUndefined();
  });
});
