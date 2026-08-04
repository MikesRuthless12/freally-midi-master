import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

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
    relatedGenres: [],
    era: null,
  },
  {
    id: 'uk-drill',
    name: 'UK Drill',
    aliases: [],
    type: 'genre',
    tier: 'standard',
    genres: [],
    relatedGenres: [],
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
function lastRequest(): { session?: Record<string, unknown>; seed?: string | null } {
  const calls = invoke.mock.calls.filter((call: unknown[]) => call[0] === 'generate_pattern');
  expect(calls.length, 'generate_pattern should have been invoked').toBeGreaterThan(0);
  const [, args] = calls[calls.length - 1] as [
    string,
    { request: { session?: Record<string, unknown>; seed?: string | null } },
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
    mood: null,
    audioEnabled: true,
    mutedLanes: [],
    edited: false,
    pins: NO_PINS,
    defaults: null,
    pendingArtist: null,
    seed: '',
    bars: 4,
    generating: false,
    error: null,
  });
});

describe('auto-sync', () => {
  // ⛔ Two things have to be arranged before a save can happen at all, and both
  // are the production behaviour rather than test scaffolding:
  //
  // 1. `persist()` is plugin-only — a browser has
  //    nowhere to put this — and `isPlugin()` detects the plugin by the
  //    `sendToPlugin` marker its webview injects.
  // 2. The write is debounced by 300 ms, because the seed box saves on every
  //    keystroke. Without controlling the clock this asserts nothing.
  beforeEach(() => {
    (window as unknown as { sendToPlugin?: () => void }).sendToPlugin = () => {};
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    delete (window as unknown as { sendToPlugin?: () => void }).sendToPlugin;
  });

  /** The session the last `save_session_state` carried. */
  function lastSaved(): Record<string, unknown> {
    const calls = invoke.mock.calls.filter(
      (call: unknown[]) => call[0] === 'save_session_state',
    );
    expect(calls.length, 'save_session_state should have been invoked').toBeGreaterThan(0);
    const [, args] = calls[calls.length - 1] as [string, { session: Record<string, unknown> }];
    return args.session;
  }

  it('is on to begin with, because following the DAW is the point', () => {
    expect(useSession.getState().autoSync).toBe(true);
  });

  it('is saved with the project, so turning it off survives a reopen', () => {
    // ⛔ It has to be in the *payload*, not merely in the store. A toggle the
    // page holds and never sends is one that silently resets on reload, and the
    // plugin would go on following the host while the chip said otherwise.
    useSession.getState().setAutoSync(false);
    vi.advanceTimersByTime(400);
    expect(lastSaved().autoSync).toBe(false);

    useSession.getState().setAutoSync(true);
    vi.advanceTimersByTime(400);
    expect(lastSaved().autoSync).toBe(true);
  });

  // FMM-S02. Same `lastSaved` harness, because the thing that has to hold is
  // the same one: a mute the page holds and never sends is a lane that comes
  // back audible on reopen with nothing saying why.
  describe('per-lane preview mute', () => {
    it('is saved with the project, and clearing the last one is expressible', () => {
      useSession.getState().setLaneMuted('snare', true);
      vi.advanceTimersByTime(400);
      expect(lastSaved().mutedLanes).toEqual(['snare']);

      // ⛔ The unmute that empties the list is the case the bridge had to be
      // written around: an empty array must reach the plugin as an empty array
      // rather than as an absent field, or the last mute can never be lifted.
      useSession.getState().setLaneMuted('snare', false);
      vi.advanceTimersByTime(400);
      expect(lastSaved().mutedLanes).toEqual([]);
    });

    it('is a set rather than a click order, so the same mutes save the same bytes', () => {
      useSession.getState().setLaneMuted('snare', true);
      useSession.getState().setLaneMuted('kick', true);
      vi.advanceTimersByTime(400);
      expect(lastSaved().mutedLanes).toEqual(['kick', 'snare']);
    });

    it('records nothing when the lane is already in that state', () => {
      useSession.getState().setLaneMuted('kick', true);
      const before = useSession.getState().mutedLanes;
      useSession.getState().setLaneMuted('kick', true);
      // Reference equality: a fresh array would be a fresh undo entry and a
      // fresh save for a click that changed nothing.
      expect(useSession.getState().mutedLanes).toBe(before);
    });
  });

  /**
   * TASK-041's persist gate: an edited clip becomes document state.
   *
   * The rest of this file is about saving the *request* — artist, seed, pins —
   * because the engine is deterministic and a few hundred bytes reopen the same
   * pattern. Editing is the one thing that breaks that property, and these are
   * the two halves of the answer: an unedited session still stores no notes,
   * and an edited one stores the clip.
   */
  describe('an edited clip', () => {
    const edit = () => ({
      ...PATTERN,
      lanes: [
        {
          lane: 'melody' as const,
          notes: [{ startTick: 0, lenTicks: 240, pitch: 61, vel: 90 }],
        },
      ],
    });

    it('is not saved at all until something has been edited', () => {
      useSession.setState({ pattern: PATTERN, edited: false });
      useSession.getState().setAutoSync(false);
      vi.advanceTimersByTime(400);
      expect(lastSaved().pattern).toBeUndefined();
      expect(lastSaved().edited).toBe(false);
    });

    it('is saved whole once the seed no longer describes it, and on every edit after', () => {
      useSession.setState({ pattern: PATTERN });
      useSession.getState().editPattern(edit());
      vi.advanceTimersByTime(400);
      expect(lastSaved().pattern).toEqual(edit());

      // ⛔ The second edit is the one that used to be lost: `edited` has
      // already flipped, so nothing in `SAVED_FIELDS` changes from here on.
      const again = { ...edit(), bars: 8 };
      useSession.getState().editPattern(again);
      vi.advanceTimersByTime(400);

      expect(lastSaved().edited).toBe(true);
      expect(lastSaved().pattern).toEqual(again);
    });

    it('goes back to storing the request when a fresh pattern is generated', () => {
      // ⛔ Otherwise a session stays "edited" for the rest of its life and every
      // save from then on carries a clip the seed already describes exactly.
      useSession.getState().select('trap');
      useSession.getState().editPattern(edit());
      expect(useSession.getState().edited).toBe(true);
      return useSession
        .getState()
        .generate()
        .then(() => {
          expect(useSession.getState().edited).toBe(false);
          vi.advanceTimersByTime(400);
          expect(lastSaved().pattern).toBeUndefined();
        });
    });
  });
});

describe('going back to a seed', () => {
  it('sends the seed exactly as typed, so a known number reproduces its beat', async () => {
    // US-004, and the reason the chip is an input rather than a readout: type a
    // seed you kept, press Generate, get that beat back. A `u64` is up to 20
    // digits and must survive as a string — `Number` would silently round the
    // ones that matter, which is a different beat with no way to tell.
    useSession.getState().select('trap');
    useSession.getState().setSeed('18446744073709551615');
    await useSession.getState().generate();

    expect(lastRequest().seed).toBe('18446744073709551615');
  });

  it('sends an empty box as absent rather than as a seed that cannot parse', async () => {
    // "Pick one for me" and "use the seed `''`" are different requests, and the
    // second one is an error rather than a generation.
    useSession.getState().select('trap');
    useSession.getState().setSeed('');
    await useSession.getState().generate();

    expect(lastRequest().seed).toBeNull();
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
      timeSigNum: null,
      timeSigDen: null,
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
      // ⛔ Absent, not 4/4. The meter a clip does not name is the host's
      // (TASK-041E), and sending a default here would drag a 6/8 project back
      // to common time on every Generate.
      timeSigNum: null,
      timeSigDen: null,
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

describe('applyPreset', () => {
  const PRESET = {
    selectedId: 'uk-drill',
    seed: '99',
    bars: 8,
    autoSync: false,
    pins: { bpm: 150, keyRoot: null, scale: null, swing: null },
  };

  it('clears the pattern rather than showing the previous artist under a new name', () => {
    useSession.getState().select('trap');
    useSession.setState({ pattern: PATTERN });

    useSession.getState().applyPreset(PRESET);

    // The preset carries inputs, not notes — the pattern is derived from the
    // seed on request, so until Generate runs there is nothing to show. Keeping
    // trap's beat under uk-drill's name is a readout that lies.
    expect(useSession.getState().pattern).toBeNull();
    expect(useSession.getState().selectedId).toBe('uk-drill');
    expect(useSession.getState().seed).toBe('99');
    expect(useSession.getState().bars).toBe(8);
    expect(useSession.getState().autoSync).toBe(false);
    expect(useSession.getState().pins.bpm).toBe(150);
  });

  it('lands as one undo step, not two', async () => {
    const { useHistory } = await import('./history');
    useSession.getState().select('trap');
    useHistory.getState().arm({
      selectedId: 'trap',
      seed: '',
      bars: 4,
      pins: NO_PINS,
      autoSync: true,
      pattern: null,
      mood: null,
      audioEnabled: true,
      mutedLanes: [],
      edited: false,
      song: null,
      songEdited: false,
    });

    useSession.getState().applyPreset(PRESET);

    // ⛔ The selection used to be `set` separately from the rest, so one load
    // recorded two entries and the first Ctrl+Z stepped back to a half-applied
    // preset that was never on screen.
    const undone = useHistory.getState().undo();
    expect(undone?.selectedId).toBe('trap');
    expect(undone?.seed).toBe('');
    expect(undone?.bars).toBe(4);
  });
});
