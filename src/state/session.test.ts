import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Pattern, RosterEntry, SessionDefaults } from '../lib/ipc-types';

/**
 * The session store's pin rules (FR-002).
 *
 * Driven through the store rather than the chips: what has to hold is that an
 * unpinned field stays absent, that a pinned one survives an artist change
 * until it is answered for, and that a read which comes back late cannot
 * overwrite the artist now selected. None of the three is visible in the DOM.
 */

const invoke = vi.fn();
vi.mock('../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
  isTauri: () => false,
}));

const { NO_PINS, useSession } = await import('./session');

const TRAP: SessionDefaults = {
  bpm: 140,
  keys: ['F#', 'C#'],
  scales: ['natural_minor'],
  swing: { grid: 'sixteenth', amount: 0.54 },
  halfTime: true,
};

const DRILL: SessionDefaults = {
  bpm: 142,
  keys: ['Bb'],
  scales: ['phrygian'],
  swing: { grid: 'sixteenth', amount: 0.5 },
  halfTime: true,
};

const ROSTER: RosterEntry[] = [
  {
    id: 'trap',
    name: 'Trap',
    aliases: [],
    type: 'genre',
    tier: 'standard',
    genres: [],
    era: null,
  },
  {
    id: 'uk-drill',
    name: 'UK Drill',
    aliases: [],
    type: 'genre',
    tier: 'standard',
    genres: [],
    era: null,
  },
];

const PATTERN: Pattern = {
  id: 'trap-1',
  part: 'drums',
  artistId: 'trap',
  seed: '1',
  bars: 4,
  bpm: 140,
  timeSigNum: 4,
  timeSigDen: 4,
  keyRoot: 6,
  scale: 'natural_minor',
  lanes: [],
  ppq: 960,
};

/** Let every pending promise settle. */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

/** The request `generate_pattern` was last called with. */
function lastRequest(): { session?: Record<string, unknown> } {
  const calls = invoke.mock.calls.filter((call: unknown[]) => call[0] === 'generate_pattern');
  expect(calls.length, 'generate_pattern should have been invoked').toBeGreaterThan(0);
  const [, args] = calls[calls.length - 1] as [
    string,
    { request: { session?: Record<string, unknown> } },
  ];
  return args.request;
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation((command: string, args?: unknown) => {
    if (command === 'session_defaults') {
      return Promise.resolve(
        (args as { styleId: string }).styleId === 'uk-drill' ? DRILL : TRAP,
      );
    }
    if (command === 'generate_pattern') return Promise.resolve(PATTERN);
    return Promise.resolve(null);
  });

  useSession.setState({
    roster: ROSTER,
    selectedId: null,
    pattern: null,
    pins: NO_PINS,
    defaults: null,
    pendingArtist: null,
    seed: '',
    bars: 4,
    generating: false,
    error: null,
  });
});

describe('session pins', () => {
  it('sends every unpinned field as absent rather than as a default', async () => {
    useSession.getState().select('trap');
    await useSession.getState().generate();

    expect(lastRequest().session).toEqual({
      bpm: null,
      keyRoot: null,
      scale: null,
      swing: null,
    });
  });

  it('sends a pinned value and leaves the rest to the artist', async () => {
    useSession.getState().select('trap');
    useSession.getState().setPin('bpm', 88);
    useSession.getState().setPin('scale', 'dorian');
    await useSession.getState().generate();

    expect(lastRequest().session).toEqual({
      bpm: 88,
      keyRoot: null,
      scale: 'dorian',
      swing: null,
    });
  });

  it('reads what a style asks for the moment it is selected', async () => {
    useSession.getState().select('uk-drill');
    await flush();

    expect(useSession.getState().defaults).toEqual(DRILL);
    expect(invoke).toHaveBeenCalledWith('session_defaults', { styleId: 'uk-drill' });
  });

  it('shows nothing rather than the last artist’s values while the read is in flight', async () => {
    useSession.getState().select('trap');
    await flush();
    expect(useSession.getState().defaults).toEqual(TRAP);

    useSession.getState().select('uk-drill');
    // Synchronously after the switch: trap's numbers must already be gone, or
    // the chips spend a frame attributing them to UK Drill.
    expect(useSession.getState().defaults).toBeNull();
  });

  it('leaves the chips empty when the read fails, without raising an error', async () => {
    // A readout that could not be filled in is not a failed generation, and
    // the banner under Generate says it is.
    invoke.mockImplementation(() => Promise.reject(new Error('no backend')));
    useSession.getState().select('trap');
    await flush();

    expect(useSession.getState().defaults).toBeNull();
    expect(useSession.getState().error).toBeNull();
  });

  it('ignores a read that comes back after the artist has changed', async () => {
    // Clicking down a roster starts one read per artist and they need not
    // return in order. Without the id check the *slowest* would win.
    let releaseTrap: ((defaults: SessionDefaults) => void) | undefined;
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command !== 'session_defaults') return Promise.resolve(null);
      if ((args as { styleId: string }).styleId === 'trap') {
        return new Promise<SessionDefaults>((resolve) => {
          releaseTrap = resolve;
        });
      }
      return Promise.resolve(DRILL);
    });

    useSession.getState().select('trap');
    useSession.getState().select('uk-drill');
    await flush();
    expect(useSession.getState().defaults).toEqual(DRILL);

    releaseTrap!(TRAP);
    await flush();
    expect(useSession.getState().defaults).toEqual(DRILL);
  });
});

describe('keep or adopt', () => {
  it('asks what to do when a pinned session meets a new artist', () => {
    useSession.getState().select('trap');
    useSession.getState().setPin('bpm', 88);
    useSession.getState().select('uk-drill');

    expect(useSession.getState().pendingArtist?.id).toBe('uk-drill');
    // The switch itself is not held up by the question.
    expect(useSession.getState().selectedId).toBe('uk-drill');
    expect(useSession.getState().pins.bpm).toBe(88);
  });

  it('asks nothing when there is nothing pinned', () => {
    useSession.getState().select('trap');
    useSession.getState().select('uk-drill');

    expect(useSession.getState().pendingArtist).toBeNull();
  });

  it('asks nothing on the first artist of the session', () => {
    // There was no previous artist for the pins to have come from.
    useSession.setState({ pins: { ...NO_PINS, swing: 0.6 } });
    useSession.getState().select('trap');

    expect(useSession.getState().pendingArtist).toBeNull();
  });

  it('drops every pin when the new artist is adopted', () => {
    useSession.getState().select('trap');
    useSession.getState().setPin('bpm', 88);
    useSession.getState().setPin('keyRoot', 5);
    useSession.getState().select('uk-drill');

    useSession.getState().adoptDefaults();

    expect(useSession.getState().pins).toEqual(NO_PINS);
    expect(useSession.getState().pendingArtist).toBeNull();
  });

  it('leaves the pins alone when they are kept', () => {
    useSession.getState().select('trap');
    useSession.getState().setPin('swing', 0.62);
    useSession.getState().select('uk-drill');

    useSession.getState().keepPins();

    expect(useSession.getState().pins.swing).toBe(0.62);
    expect(useSession.getState().pendingArtist).toBeNull();
  });

  it('unpins one field without disturbing the others', () => {
    useSession.getState().select('trap');
    useSession.getState().setPin('bpm', 88);
    useSession.getState().setPin('swing', 0.62);
    useSession.getState().setPin('bpm', null);

    expect(useSession.getState().pins).toEqual({ ...NO_PINS, swing: 0.62 });
  });
});
