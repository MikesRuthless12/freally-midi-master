import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';

vi.mock('../../lib/ipc', () => ({ invoke: vi.fn() }));

const { Announcer, ZERO_WIDTH } = await import('./Announcer');
const { useSession } = await import('../../state/session');

/**
 * What the one sentence a screen reader hears actually says (TASK-095).
 *
 * ⛔ **Here rather than in a Playwright spec.** The live region is
 * `visually-hidden` and its whole job is to change *between* two generations —
 * asserting that from a browser means asserting on text nobody can see, twice,
 * with a generation in between. What has to hold is a rule about the sentence,
 * and the sentence is built from store state.
 */

afterEach(() => {
  cleanup();
  useSession.setState({ patterns: {}, generating: false });
});

const PATTERN = {
  id: 'p1',
  part: 'chords',
  seed: 'seed-a',
  songSeed: 'song',
  bars: 4,
  bpm: 140,
  timeSigNum: 4,
  timeSigDen: 4,
  keyRoot: 7,
  scale: 'minor',
  lanes: [],
  ppq: 960,
};

function landed(over: Record<string, unknown> = {}) {
  act(() => {
    useSession.setState({
      generating: false,
      patterns: { chords: { ...PATTERN, ...over } } as never,
    });
  });
}

const spoken = () => (screen.getByRole('status').textContent ?? '').split(ZERO_WIDTH).join('');

/**
 * ⛔⛔ **`keyRoot` is a PITCH CLASS.** Interpolated raw, the sentence said
 * *"7 minor"* for G minor — a reader told something the chips are not saying.
 */
it('names the key as a note, not as the pitch class integer', () => {
  landed();
  render(<Announcer />);
  expect(spoken()).toContain('G minor');
  expect(spoken()).not.toMatch(/\b7\b/);
});

/**
 * ⛔ **Setting state to the same string is a React bail-out**, so the second of
 * two identical generations changed no text node and was never spoken. The
 * zero-width space is what makes the DOM differ; a reader ignores it.
 */
it('speaks again when a second generation reads identically', () => {
  landed();
  render(<Announcer />);
  const first = screen.getByRole('status').textContent ?? '';
  expect(first).not.toBe('');

  // Same bars, tempo and key — only the seed differs, and the seed is not in
  // the copy, so the two sentences are the same string.
  landed({ seed: 'seed-b' });
  const second = screen.getByRole('status').textContent ?? '';
  expect(second).not.toBe(first);
  expect(spoken()).toBe(first.split(ZERO_WIDTH).join(''));
});
