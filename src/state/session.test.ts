import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  Lane,
  Part,
  Pattern,
  RosterEntry,
  SessionDefaults,
  SplitPart,
} from '../lib/ipc-types';

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

const { BAR_CHOICES, NO_PINS, mirrorableDrumsSeed, useSession } = await import('./session');
const { useVariations } = await import('./variations');
const { useUi } = await import('./ui');

const TRAP: SessionDefaults = {
  bpm: 140,
  bpmMin: 132,
  bpmMax: 148,
  // ⚠ Four of five, so a fixture cannot make the detail pane's "does not write"
  // line untestable by claiming everything (TASK-158D).
  parts: ['drums', 'chords', 'melody', 'bass'],
  keys: ['F#', 'C#'],
  scales: ['natural_minor'],
  swing: { grid: 'sixteenth', amount: 0.54 },
  halfTime: true,
};

const DRILL: SessionDefaults = {
  bpm: 142,
  bpmMin: 138,
  bpmMax: 146,
  parts: ['drums', 'melody', 'bass'],
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
    mine: false,
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
    mine: false,
  },
];

const PATTERN: Pattern = {
  id: 'trap-1',
  part: 'drums',
  artistId: 'trap',
  seed: '1',
  songSeed: '1',
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
function lastRequest(): {
  session?: Record<string, unknown>;
  seed?: string | null;
  songSeed?: string | null;
} {
  const calls = invoke.mock.calls.filter((call: unknown[]) => call[0] === 'generate_pattern');
  expect(calls.length, 'generate_pattern should have been invoked').toBeGreaterThan(0);
  const [, args] = calls[calls.length - 1] as [
    string,
    {
      request: {
        session?: Record<string, unknown>;
        seed?: string | null;
        songSeed?: string | null;
      };
    },
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
    patterns: {},
    editedParts: [],
    mood: null,
    base: null,
    audioEnabled: true,
    mutedLanes: [],
    soloedLanes: [],
    lockedLanes: [],
    edited: false,
    pins: NO_PINS,
    defaults: null,
    pendingArtist: null,
    seed: '',
    songSeed: '',
    seedPinned: false,
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

  describe('per-lane locks and reroll (TASK-044)', () => {
    /** A drum pattern whose kick and hat both differ per seed. */
    const take = (seed: string): Pattern => ({
      id: `p-${seed}`,
      part: 'drums',
      artistId: 'trap',
      seed,
      songSeed: seed,
      bars: 4,
      bpm: 140,
      timeSigNum: 4,
      timeSigDen: 4,
      keyRoot: 6,
      scale: 'natural_minor',
      ppq: 960,
      lanes: [
        {
          lane: 'kick',
          notes: [{ startTick: Number(seed), lenTicks: 120, pitch: 36, vel: 100 }],
        },
        {
          lane: 'closedHat',
          notes: [{ startTick: Number(seed) + 5, lenTicks: 120, pitch: 42, vel: 90 }],
        },
      ],
    });

    // ⛔ `generate` returns early with no artist selected, which is the
    // production guard rather than scaffolding — a Generate with nothing chosen
    // is not a thing to answer.
    beforeEach(() => {
      useSession.setState({ selectedId: 'trap', patterns: {}, lockedLanes: [] });
    });

    async function generateReturning(seed: string) {
      invoke.mockImplementation((command: string) =>
        command === 'generate_pattern' ? Promise.resolve(take(seed)) : Promise.resolve(null),
      );
      await useSession.getState().generate('drums');
    }

    it('holds a locked lane byte-for-byte across ten rerolls', async () => {
      // ⛔ The roadmap's verify line: lock the kick lane, reroll 10x, kick
      // identical. Ten *different* answers from the engine, so a splice that
      // did nothing would fail on the first one.
      await generateReturning('100');
      const kick = useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'kick');
      expect(kick).toBeDefined();

      useSession.getState().setLaneLocked('kick', true);

      for (let i = 0; i < 10; i += 1) {
        await generateReturning(String(200 + i * 37));
        const pattern = useSession.getState().patterns.drums;
        // Byte-identical, and the *same object* — the splice keeps the track
        // rather than rebuilding an equal one, which is what makes it exact.
        expect(pattern?.lanes.find((l) => l.lane === 'kick')).toBe(kick);
        // ...while the unlocked lane really did reroll.
        expect(pattern?.lanes.find((l) => l.lane === 'closedHat')?.notes[0].startTick).toBe(
          200 + i * 37 + 5,
        );
      }
    });

    it('lets the lane go again when it is unlocked', async () => {
      await generateReturning('100');
      useSession.getState().setLaneLocked('kick', true);
      await generateReturning('300');
      expect(
        useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'kick')?.notes[0]
          .startTick,
      ).toBe(100);

      useSession.getState().setLaneLocked('kick', false);
      await generateReturning('400');
      expect(
        useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'kick')?.notes[0]
          .startTick,
      ).toBe(400);
    });

    it('holds a locked lane through Generate All, not just Generate', async () => {
      // ⛔⛔ **The regression this exists for.** `withLocks` was spliced inside
      // `generate`'s updater only, so locking a lane and pressing Shift+G —
      // `generateAll`, a gesture added in the same session — threw the lock
      // away without a word. A rule installed at one door rather than at the
      // seam both doors go through.
      //
      // ⚠ **Proved here rather than in Playwright, and that is the finding.**
      // The first attempt was an e2e, and it **passed with the fix reverted**:
      // the browser mock answers with one fixed seed, so the kick is identical
      // whether the lock applied or not. Only a mock that varies per call can
      // tell a held lane from an unchanged one.
      await generateReturning('100');
      const kick = useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'kick');
      useSession.getState().setLaneLocked('kick', true);

      // Every part answers with a *different* take, so an unheld kick moves.
      let call = 0;
      invoke.mockImplementation((command: string) => {
        if (command !== 'generate_pattern') return Promise.resolve(null);
        call += 1;
        return Promise.resolve(take(String(500 + call * 11)));
      });
      await useSession.getState().generateAll();

      expect(useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'kick')).toBe(
        kick,
      );
      // ...and the unlocked lane in the same clip really did reroll, so this
      // cannot pass by Generate All having done nothing at all.
      expect(
        useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'closedHat')
          ?.notes[0].startTick,
      ).not.toBe(105);
    });

    it('keeps a held part in the project, because the seed can no longer make it', async () => {
      // ⛔⛔ **The lock survived the reroll and then vanished from the file.**
      // `generate` cleared the part from `editedParts` unconditionally — "a
      // fresh generation *is* the seed's own output again" — which stopped
      // being true the moment `withLocks` began splicing a previous take's lane
      // into it. `send()` reads `edited` to decide whether *any* clip is
      // written, so a producer who drew hats, locked them and pressed Generate
      // saved a project holding no clips at all: reopening regenerated from the
      // seed and the lane they had deliberately kept was gone, silently.
      await generateReturning('100');
      useSession.getState().setLaneLocked('kick', true);
      await generateReturning('300');

      expect(useSession.getState().editedParts).toContain('drums');
      expect(useSession.getState().edited).toBe(true);

      // ...and it goes back to being reproducible once the lock comes off, or
      // every project from then on would carry clips it does not need.
      useSession.getState().setLaneLocked('kick', false);
      await generateReturning('400');
      expect(useSession.getState().editedParts).not.toContain('drums');
      expect(useSession.getState().edited).toBe(false);
    });

    it('does not carry a locked lane past the end of a shorter clip', async () => {
      // ⛔⛔ **Invisible on screen and audible in the export.** `setBars` does
      // not clear `patterns`, so locking the kick on eight bars and generating
      // at four spliced the whole eight-bar track into a four-bar clip.
      // `toCells` and `columnDensity` both bounds-check, so the grid drew a
      // clean four bars while the notes were still in `lanes` — and went to the
      // host, to `to_midi` and to `stem_files`. The stem played in bars five to
      // eight, and nothing on screen ever showed it.
      const long = take('0');
      long.bars = 8;
      long.lanes[0].notes = [
        { startTick: 0, lenTicks: 120, pitch: 36, vel: 100 },
        // Bar 5 of eight: inside the long clip, past the end of a four-bar one.
        { startTick: 960 * 16, lenTicks: 120, pitch: 36, vel: 100 },
      ];
      invoke.mockImplementation((command: string) =>
        command === 'generate_pattern' ? Promise.resolve(long) : Promise.resolve(null),
      );
      await useSession.getState().generate('drums');
      useSession.getState().setLaneLocked('kick', true);

      await generateReturning('300'); // `take` is four bars.
      const kick = useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'kick');
      expect(kick?.notes.map((n) => n.startTick)).toEqual([0]);
    });

    it('recalls the take that was made, not a hybrid of it and the current locks', async () => {
      // ⛔⛔ **A clip that never existed, labelled as one that did.** Recall
      // regenerates — that is what makes an entry tens of bytes — but it went
      // through `generate`, which splices whatever is locked *now* into the
      // answer. So going back three takes returned take 1's hats under take 3's
      // kick, while `VariationNav` read "1 / 3". A history that reports a take
      // it did not give you is worse than no history.
      await generateReturning('100');
      const first = useVariations.getState().entries.drums[0];

      await generateReturning('700');
      useSession.getState().setLaneLocked('kick', true);

      // Recall asks the engine again; answer with the original take, as a
      // deterministic engine would for the same seed.
      invoke.mockImplementation((command: string) =>
        command === 'generate_pattern' ? Promise.resolve(take('100')) : Promise.resolve(null),
      );
      await useSession.getState().recallVariation(first);

      const kick = useSession.getState().patterns.drums?.lanes.find((l) => l.lane === 'kick');
      expect(kick?.notes[0].startTick).toBe(100);
    });

    it('recalls the tempo and key the take was actually written at', async () => {
      // ⛔⛔ **The readout and the notes disagreeing about the same take.** The
      // entry stores the *pins* and, separately, the `bpm`/`keyRoot`/`scale`/
      // meter that were **resolved** — which is the whole reason it stores
      // both. Recall restored only the pins, so a take made at 140 came back at
      // whatever the session had drifted to while the nav went on displaying
      // 140 off the entry.
      await generateReturning('100');
      const entry = useVariations.getState().entries.drums[0];
      expect(entry.bpm).toBe(140);

      // The producer moves on and changes the tempo.
      useSession.getState().setPin('bpm', 90);
      await useSession.getState().recallVariation(entry);

      const pins = useSession.getState().pins;
      expect(pins.bpm).toBe(entry.bpm);
      expect(pins.keyRoot).toBe(entry.keyRoot);
      expect(pins.scale).toBe(entry.scale);
      expect(pins.timeSigNum).toBe(entry.timeSigNum);
      expect(pins.timeSigDen).toBe(entry.timeSigDen);
    });

    it('reloads the artist’s own defaults when a recall changes artists', async () => {
      // ⛔⛔ **The roster highlighted one artist and the pane described
      // another.** Recall wrote `selectedId` with a bare `set`, so `defaults` —
      // what the ARTIST pane's "tends to" line reads, and what every unpinned
      // field falls back on — still held the previous artist's. The other four
      // part slots were left up too, showing that artist's clips under this
      // one's name.
      await generateReturning('100');
      const trapTake = useVariations.getState().entries.drums[0];

      useSession.setState({
        selectedId: 'uk-drill',
        defaults: DRILL,
        patterns: { melody: take('9') },
      });

      invoke.mockImplementation((command: string) => {
        if (command === 'generate_pattern') return Promise.resolve(take('100'));
        if (command === 'session_defaults') return Promise.resolve(TRAP);
        return Promise.resolve(null);
      });
      await useSession.getState().recallVariation(trapTake);

      expect(useSession.getState().selectedId).toBe('trap');
      expect(useSession.getState().defaults).toEqual(TRAP);
      // The other artist's clips are gone rather than sitting under this name.
      expect(useSession.getState().patterns.melody).toBeUndefined();
    });

    it('is saved with the project, sorted, and records no step for a no-op', () => {
      useSession.getState().setLaneLocked('closedHat', true);
      useSession.getState().setLaneLocked('kick', true);
      vi.advanceTimersByTime(400);
      expect(lastSaved().lockedLanes).toEqual(['closedHat', 'kick']);

      const before = useSession.getState().lockedLanes;
      useSession.getState().setLaneLocked('kick', true);
      expect(useSession.getState().lockedLanes).toBe(before);
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
      useSession.setState({ patterns: { drums: PATTERN }, edited: false });
      useSession.getState().setAutoSync(false);
      vi.advanceTimersByTime(400);
      expect(lastSaved().patterns).toBeUndefined();
      expect(lastSaved().edited).toBe(false);
    });

    it('is saved whole once the seed no longer describes it, and on every edit after', () => {
      useSession.setState({ patterns: { drums: PATTERN } });
      useSession.getState().editPattern(edit());
      vi.advanceTimersByTime(400);
      expect(lastSaved().patterns).toEqual({ drums: edit() });

      // ⛔ The second edit is the one that used to be lost: `edited` has
      // already flipped, so nothing in `SAVED_FIELDS` changes from here on.
      const again = { ...edit(), bars: 8 };
      useSession.getState().editPattern(again);
      vi.advanceTimersByTime(400);

      expect(lastSaved().edited).toBe(true);
      expect(lastSaved().patterns).toEqual({ drums: again });
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
          expect(lastSaved().patterns).toBeUndefined();
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

  // ── TASK-141: the record is carried, the take is not ────────────────────
  it('starts a record on the first Generate and carries it to every part after', async () => {
    // ⛔⛔ **This is the whole of TASK-141 seen from the page.** The engine
    // guarantees five parts agree only when they share a harmonic plan, and the
    // Defect 2 fix made every Generate roll a fresh seed — so the ordinary
    // workflow (Generate on Drums, switch tab, Generate on Melody) wrote the
    // melody against a progression the chords tab had never seen. Both clips
    // looked individually correct, which is exactly why nothing caught it.
    useSession.getState().select('trap');

    // Nothing generated yet, so this one starts the record.
    await useSession.getState().generate('drums');
    expect(lastRequest().songSeed, 'the first Generate has no record to join').toBeNull();

    const record = useSession.getState().songSeed;
    expect(record, 'the engine answers with the record it chose').not.toBe('');

    // Every part after it joins that record rather than starting its own.
    for (const part of ['melody', 'chords', 'bass'] as const) {
      await useSession.getState().generate(part);
      expect(lastRequest().songSeed, `${part} must join the record`).toBe(record);
    }
    expect(useSession.getState().songSeed).toBe(record);
  });

  it('carries the record even while the take is unpinned and rerolling', async () => {
    // ⚠ The two are deliberately independent: the seed lock is about the
    // producer's typed *take*, and the record is carried whether or not
    // anything is pinned. A producer should not have to know the song seed
    // exists to get parts that belong together.
    useSession.getState().select('trap');
    useSession.getState().setSeed('');
    await useSession.getState().generate('drums');

    const record = useSession.getState().songSeed;
    expect(useSession.getState().seedPinned, 'the take is not pinned').toBe(false);

    await useSession.getState().generate('melody');
    expect(lastRequest().seed, 'the take still rerolls').toBeNull();
    expect(lastRequest().songSeed, 'the record still travels').toBe(record);
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
    songSeed: '99',
    bars: 8,
    autoSync: false,
    pins: { bpm: 150, keyRoot: null, scale: null, swing: null },
  };

  it('clears the pattern rather than showing the previous artist under a new name', () => {
    useSession.getState().select('trap');
    useSession.setState({ patterns: { drums: PATTERN } });

    useSession.getState().applyPreset(PRESET);

    // The preset carries inputs, not notes — the pattern is derived from the
    // seed on request, so until Generate runs there is nothing to show. Keeping
    // trap's beat under uk-drill's name is a readout that lies.
    expect(useSession.getState().patterns).toEqual({});
    expect(useSession.getState().selectedId).toBe('uk-drill');
    expect(useSession.getState().seed).toBe('99');
    expect(useSession.getState().bars).toBe(8);
    expect(useSession.getState().autoSync).toBe(false);
    expect(useSession.getState().pins.bpm).toBe(150);
  });

  /**
   * What a stored seed means when the project does not say (2026-08-06).
   *
   * ⛔ **Absent has to mean pinned, and the direction matters.** Every project
   * written before the seed could be unpinned re-sent its stored seed on every
   * Generate, so reading absence as *unpinned* would reopen all of them rolling
   * a new beat — US-004's promise broken by an upgrade, silently, on work
   * somebody saved. A stored `false` is honoured as itself, because that
   * session genuinely never chose the seed it is showing.
   */
  it('treats a seed stored before the pin existed as the producer’s own', () => {
    useSession.getState().applyPreset(PRESET);
    expect(useSession.getState().seedPinned).toBe(true);
  });

  it('honours a stored unpinned seed rather than re-pinning it', () => {
    useSession.getState().applyPreset({ ...PRESET, seedPinned: false });
    expect(useSession.getState().seedPinned).toBe(false);
    // Still shown, so it can be read and copied — it is a readout, not a choice.
    expect(useSession.getState().seed).toBe('99');
  });

  it('never pins an empty seed, whatever the project claims', () => {
    useSession.getState().applyPreset({ ...PRESET, seed: '', seedPinned: true });
    expect(useSession.getState().seedPinned).toBe(false);
  });

  it('lands as one undo step, not two', async () => {
    const { useHistory } = await import('./history');
    useSession.getState().select('trap');
    useHistory.getState().arm({
      selectedId: 'trap',
      seed: '',
      songSeed: '',
      seedPinned: false,
      bars: 4,
      pins: NO_PINS,
      autoSync: true,
      patterns: {},
      editedParts: [],
      mood: null,
      base: null,
      audioEnabled: true,
      mutedLanes: [],
      soloedLanes: [],
      lockedLanes: [],
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

/**
 * The five generators each keep their own clip (TASK-119).
 *
 * ⛔ **The defect this closes was documented rather than unknown.** The store
 * held one `pattern`, so `CenterStage` drew a tab's editor only when the slot
 * happened to contain that tab's part — and generating a bassline threw the
 * melody away. Mike found it in FL Studio; no gate could, because "the other
 * four parts should still be there" was never asserted anywhere.
 */
describe('each part keeps its own pattern', () => {
  beforeEach(() => {
    // Answer with the part that was asked for, which the shared mock does not:
    // a single fixed `PATTERN` would pass this test by accident.
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === 'session_defaults') return Promise.resolve(TRAP);
      if (command === 'generate_pattern') {
        const { part, seed } = (args as { request: { part: Part; seed: string | null } })
          .request;
        return Promise.resolve({
          ...PATTERN,
          id: `trap-${part}`,
          part,
          seed: seed ?? '77',
          songSeed: seed ?? '77',
        } satisfies Pattern);
      }
      return Promise.resolve(null);
    });
    useSession.setState({ selectedId: 'trap', patterns: {}, seed: '' });
  });

  it('generating a bassline leaves the melody where it was', async () => {
    await useSession.getState().generate('melody');
    await useSession.getState().generate('bass');

    const { patterns } = useSession.getState();
    expect(patterns.melody?.part, 'the melody was destroyed by generating a bass').toBe(
      'melody',
    );
    expect(patterns.bass?.part).toBe('bass');
  });

  it('fills every slot as the parts are generated, and replaces only its own', async () => {
    for (const part of ['drums', 'melody', 'counter', 'bass', 'chords'] as const) {
      await useSession.getState().generate(part);
    }
    expect(Object.keys(useSession.getState().patterns).sort()).toEqual([
      'bass',
      'chords',
      'counter',
      'drums',
      'melody',
    ]);

    const before = useSession.getState().patterns;
    await useSession.getState().generate('drums');
    const after = useSession.getState().patterns;

    // Regenerating one part replaces that one and leaves the other four
    // *identical by reference* — a fresh object for an untouched part would
    // re-render its editor and break the undo stack's shared-reference model.
    expect(after.drums).not.toBe(before.drums);
    expect(after.melody).toBe(before.melody);
    expect(after.chords).toBe(before.chords);
  });

  it('reuses a PINNED seed across parts, because coherence depends on it', async () => {
    // ⛔ `engine/src/parts.rs` guarantees five parts agree only when they share
    // a seed. With one slot nobody could tell; with five, drawing a fresh seed
    // per part would produce five clips in the same key that fit nothing.
    //
    // ⚠ **The pin is what carries this now, and that is the deliberate change
    // of 2026-08-06.** It used to ride on the seed the engine echoed back —
    // which also meant a second press of Generate on *one* part reproduced the
    // beat it had just made, forever. The mechanism is the same; what changed
    // is that the producer says when they want it.
    useSession.getState().setSeed('4242');
    await useSession.getState().generate('melody');
    expect(lastRequest().seed).toBe('4242');

    await useSession.getState().generate('bass');
    expect(lastRequest().seed).toBe('4242');
  });

  it('asks for a fresh seed on every press while the seed is unpinned', async () => {
    // ⛔⛔ **The defect Mike found in Ableton on 2026-08-06**, at the level it
    // was caused: *"the seed stayed the same and there was no variation"*. The
    // engine's reply is echoed into the box so it can be read and copied, and
    // the old code then re-sent it — so the second press and every press after
    // it regenerated the first beat.
    //
    // ⚠ Asserted on what was **sent**, not on what came back. The mock answers
    // with whatever it is given, so a test that only compared the resulting
    // patterns would pass against the broken code.
    await useSession.getState().generate('drums');
    expect(useSession.getState().seed, 'the engine picked and it was shown').toBe('77');

    await useSession.getState().generate('drums');
    expect(
      lastRequest().seed,
      'the echoed seed was re-sent, so Generate reproduced its own beat',
    ).toBeNull();
  });

  it('does not pin the seed it echoes back, however many times it is pressed', async () => {
    for (let press = 0; press < 3; press += 1) {
      await useSession.getState().generate('drums');
    }
    expect(useSession.getState().seedPinned).toBe(false);

    const sent = invoke.mock.calls
      .filter((call: unknown[]) => call[0] === 'generate_pattern')
      .map((call: unknown[]) => (call[1] as { request: { seed: string | null } }).request.seed);
    expect(sent, 'a press after the first named a seed instead of asking for one').toEqual([
      null,
      null,
      null,
    ]);
  });

  it('holds the seed once the producer locks what came back', async () => {
    // The other half of the affordance: like what you just got, keep it —
    // without retyping the twenty digits already on screen.
    await useSession.getState().generate('drums');
    const got = useSession.getState().seed;

    useSession.getState().setSeedPinned(true);
    await useSession.getState().generate('drums');

    expect(lastRequest().seed).toBe(got);
  });

  // ── A style's own samples come back with it (TASK-049) ──────────────────
  it('loads a style’s copied samples when the producer selects it', async () => {
    // ⛔⛔ **The half that was missing, and it made the consent text false.** The
    // checkbox promises the copies *"still work if you move or delete the
    // originals"*, and nothing read `models/<id>/samples/` back — Mike found it
    // by hand: clear the kick on a saved style, select something else, come
    // back, and the kick did not return.
    useSession.setState({
      roster: [
        ...ROSTER,
        {
          id: 'my-edm',
          name: 'My EDM',
          aliases: [],
          type: 'artist',
          tier: 'standard',
          genres: [],
          relatedGenres: [],
          era: null,
          mine: true,
        },
      ],
      selectedId: 'trap',
    });

    useSession.getState().select('my-edm');
    await vi.waitFor(() =>
      expect(
        invoke.mock.calls.filter((call: unknown[]) => call[0] === 'user_model_load_samples')
          .length,
      ).toBe(1),
    );
    const [, args] = invoke.mock.calls.find(
      (call: unknown[]) => call[0] === 'user_model_load_samples',
    ) as [string, { id: string }];
    expect(args.id).toBe('my-edm');
  });

  it('asks nothing of the bridge when the style is not the producer’s own', async () => {
    // ⛔ A shipped artist has no copied samples by construction. Asking anyway
    // would be a round trip per click through a five-hundred-name roster, to be
    // told no every time — and the `mine` check is what makes the read-back cost
    // nothing for everybody who never built a style.
    useSession.setState({ roster: ROSTER, selectedId: 'trap' });

    useSession.getState().select('uk-drill');
    await Promise.resolve();

    expect(
      invoke.mock.calls.filter((call: unknown[]) => call[0] === 'user_model_load_samples'),
    ).toEqual([]);
  });

  it('refuses to lock an empty box, so the flag cannot contradict the seed', () => {
    useSession.getState().setSeed('');
    useSession.getState().setSeedPinned(true);
    expect(useSession.getState().seedPinned).toBe(false);
  });

  it('unpins when the box is cleared, which is how you ask for a surprise', () => {
    useSession.getState().setSeed('4242');
    expect(useSession.getState().seedPinned).toBe(true);

    useSession.getState().setSeed('');
    expect(useSession.getState().seedPinned).toBe(false);
  });

  // ── Generate all five at once (TASK-120) ────────────────────────────────
  it('fills every part from one seed', async () => {
    await useSession.getState().generateAll();

    const { patterns, seed } = useSession.getState();
    expect(Object.keys(patterns).sort()).toEqual([
      'bass',
      'chords',
      'counter',
      'drums',
      'melody',
    ]);

    // ⛔ **The claim that makes them a record rather than five loops.** The seed
    // box started empty, so the engine picked — and every part after the first
    // had to be given the same one. `parts.rs` guarantees coherence on a shared
    // seed and guarantees nothing otherwise.
    const seeds = new Set(Object.values(patterns).map((p) => p!.seed));
    expect(seeds.size, 'the five parts were generated from different seeds').toBe(1);
    expect(seed).toBe([...seeds][0]);
  });

  it('sends the same seed on all five requests, not just the last', async () => {
    await useSession.getState().generateAll();

    const sent = invoke.mock.calls
      .filter((call: unknown[]) => call[0] === 'generate_pattern')
      .map((call: unknown[]) => (call[1] as { request: { seed: string | null } }).request.seed);

    expect(sent).toHaveLength(5);
    // The first asks for one ("pick for me"); the rest must name it.
    expect(sent[0]).toBeNull();
    expect(new Set(sent.slice(1)).size).toBe(1);
    expect(sent[1]).toBe(useSession.getState().seed);
  });

  it('honours a seed the producer typed, without asking for a new one', async () => {
    // ⚠ Through `setSeed`, not a bare `setState`: typing is what pins, and a
    // seed the store merely *holds* is the engine's echo, which `generateAll`
    // must roll past rather than rebuild the same record from.
    useSession.getState().setSeed('4242');
    await useSession.getState().generateAll();

    const sent = invoke.mock.calls
      .filter((call: unknown[]) => call[0] === 'generate_pattern')
      .map((call: unknown[]) => (call[1] as { request: { seed: string | null } }).request.seed);

    expect(sent).toEqual(['4242', '4242', '4242', '4242', '4242']);
  });

  it('draws a fresh seed for the set on every unpinned press of Generate all', async () => {
    // ⛔ The same defect as the single-part case, one document up: the first run
    // echoes its seed into the box, and starting the second run from that box
    // rebuilt the identical five-part record. Mike asked for a new one *"every
    // time i click 'Generate' or 'Generate All'"*, and named both.
    await useSession.getState().generateAll();
    const after = invoke.mock.calls.filter(
      (call: unknown[]) => call[0] === 'generate_pattern',
    ).length;

    await useSession.getState().generateAll();

    const second = invoke.mock.calls
      .filter((call: unknown[]) => call[0] === 'generate_pattern')
      .slice(after)
      .map((call: unknown[]) => (call[1] as { request: { seed: string | null } }).request.seed);

    // ⚠ First asks, and the remaining four still share what it answered — the
    // `parts.rs` rule is untouched. What must not happen is the *first* one
    // naming the seed the previous run ended on.
    expect(second[0], 'the second run rebuilt the first run’s record').toBeNull();
    expect(new Set(second.slice(1)).size).toBe(1);
  });

  // ── Clear, per part and for all (TASK-121) ──────────────────────────────
  it('clears one part and leaves the other four', async () => {
    await useSession.getState().generateAll();
    useSession.getState().clearPart('melody');

    const { patterns } = useSession.getState();
    expect(patterns.melody).toBeUndefined();
    expect(Object.keys(patterns).sort()).toEqual(['bass', 'chords', 'counter', 'drums']);
  });

  it('clears all five together', async () => {
    await useSession.getState().generateAll();
    useSession.getState().clearAll();
    expect(useSession.getState().patterns).toEqual({});
  });

  // ── The defects /code-review found in TASK-119/120 ──────────────────────
  it('keeps another part edited when one part is regenerated', async () => {
    // ⛔ **Silent, permanent project data loss.** `edited` was one flag for the
    // whole session and `send()` uses it to decide whether *any* clip is saved
    // — so generating a bassline cleared it and the next save wrote the project
    // with no clips at all, deleting a melody the producer had hand-edited.
    await useSession.getState().generate('melody');
    const edited = { ...useSession.getState().patterns.melody!, bars: 8 };
    useSession.getState().editPattern(edited);
    expect(useSession.getState().edited).toBe(true);

    await useSession.getState().generate('bass');

    expect(useSession.getState().editedParts).toEqual(['melody']);
    expect(useSession.getState().edited, 'the melody edit was forgotten').toBe(true);
    expect(useSession.getState().patterns.melody?.bars).toBe(8);
  });

  it('stops claiming to be edited once the edited part is cleared', () => {
    useSession.setState({ patterns: { drums: PATTERN }, editedParts: ['drums'], edited: true });
    useSession.getState().clearPart('drums');

    // Otherwise `send()` goes on writing purely generated clips into the
    // project, which are then replayed rather than regenerated forever after.
    expect(useSession.getState().edited).toBe(false);
    expect(useSession.getState().editedParts).toEqual([]);
  });

  it('keeps the parts that generated when a style refuses one of them', async () => {
    // ⛔ **This is Drake, and most of the trap roster.** A style whose 808 *is*
    // the bassline authors no separate bass part (FR-007), so the engine refuses
    // that request — and aborting the run there threw away the four parts that
    // had already come back. Generate all did nothing at all on a flagship.
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === 'session_defaults') return Promise.resolve(TRAP);
      if (command === 'generate_pattern') {
        const { part, seed } = (args as { request: { part: Part; seed: string | null } })
          .request;
        if (part === 'bass') {
          return Promise.reject(new Error("Drake's 808 is the bassline"));
        }
        return Promise.resolve({ ...PATTERN, id: `trap-${part}`, part, seed: seed ?? '77' });
      }
      return Promise.resolve(null);
    });

    await useSession.getState().generateAll();

    const { patterns, error } = useSession.getState();
    expect(Object.keys(patterns).sort()).toEqual(['chords', 'counter', 'drums', 'melody']);
    // ...and the refusal is still reported, rather than the producer being left
    // to notice an empty tab later.
    expect(error).toContain('bassline');
  });

  it('does not write the previous artist’s clips after the artist changes', async () => {
    // `select` clears the slots and swaps the id; a loop that ignored that wrote
    // five clips of the outgoing artist under the incoming one's name.
    let resolveFirst: ((value: Pattern) => void) | undefined;
    invoke.mockImplementation((command: string) => {
      if (command === 'session_defaults') return Promise.resolve(TRAP);
      if (command === 'generate_pattern') {
        return new Promise((resolve) => {
          resolveFirst = resolve;
        });
      }
      return Promise.resolve(null);
    });

    const running = useSession.getState().generateAll();
    await flush();
    useSession.setState({ selectedId: 'uk-drill' });
    resolveFirst?.({ ...PATTERN, part: 'drums', seed: '5' });
    await running;

    expect(useSession.getState().patterns).toEqual({});
    expect(useSession.getState().generating).toBe(false);
  });

  it('does nothing when there is nothing to clear', async () => {
    await useSession.getState().generate('drums');
    const before = useSession.getState().patterns;

    // ⚠ Reference equality: a fresh object would be a fresh undo entry and a
    // fresh save for a button press that changed nothing — the rule
    // `setLaneMuted` already follows.
    useSession.getState().clearPart('melody');
    expect(useSession.getState().patterns).toBe(before);

    useSession.getState().clearAll();
    useSession.getState().clearAll();
    expect(useSession.getState().patterns).toEqual({});
  });
});

/**
 * The Stems panel puts itself in front of a producer who has generated
 * something (Mike, 2026-08-06).
 *
 * ⛔ **The defect was that the panel remembers being collapsed across reloads**,
 * and it holds the only way to get a pattern out of the plugin. Collapse it
 * once and the drag rows are unreachable, with nothing on screen saying they
 * exist — which is exactly how Mike came to report that the drums could not be
 * dragged out per instrument when half of that was already built.
 */
describe('the Stems panel reveals itself once anything is generated', () => {
  let useUi: typeof import('./ui').useUi;

  beforeEach(async () => {
    ({ useUi } = await import('./ui'));
    // Collapsed and hidden — the state a returning producer is actually in,
    // and the one the fix has to survive.
    useUi.setState({
      stemsRevealed: false,
      rightRailOpen: false,
      sections: { ...useUi.getState().sections, stems: false },
    });
    useSession.setState({ selectedId: 'trap', patterns: {} });
  });

  it('opens the panel on the first generation of the session', async () => {
    expect(useUi.getState().sections.stems).toBe(false);

    await useSession.getState().generate('drums');

    expect(useUi.getState().sections.stems).toBe(true);
  });

  it('leaves the right rail alone, because opening it costs the editor height', async () => {
    // ⛔⛔ **The first cut of this forced the rail open and `e2e/piano-roll.
    // spec.ts:380` caught it** — the stage re-lays, the velocity lane loses
    // height, and a drag to velocity 96 lands on 85. `StemsPanel`'s header
    // records the two earlier times something near the pattern grew.
    //
    // ⚠ Nothing is lost by leaving it: the plugin's page always lays out at
    // 1440, which is `WIDE_BREAKPOINT`, so the rail is already open there.
    await useSession.getState().generate('drums');

    expect(useUi.getState().rightRailOpen).toBe(false);
  });

  it('does not reopen it after the producer has switched away again', async () => {
    // ⚠ **"Closed it" is now "switched the rail to the other group"** — a rail
    // shows one group and a panel cannot leave on its own. The property under
    // test is unchanged and is the one that matters: having been shown Stems
    // once, the app must not go on yanking the rail back to it.
    await useSession.getState().generate('drums');
    useUi.getState().showSection('session');
    expect(useUi.getState().sections.stems).toBe(false);

    // ⚠ The subscriber runs on every write that leaves a pattern in the store,
    // so without the one-shot this would fight the producer on every press.
    await useSession.getState().generate('melody');
    await useSession.getState().generateAll();

    expect(useUi.getState().sections.stems).toBe(false);
  });

  it('stays shut while nothing has been generated', () => {
    // Selecting an artist is not generating, and a panel that opened on
    // browsing would be back to being noise.
    useSession.getState().select('uk-drill');

    expect(useUi.getState().sections.stems).toBe(false);
    expect(useUi.getState().stemsRevealed).toBe(false);
  });
});

/**
 * What lengths a generation is offered at (Mike, 2026-08-06).
 *
 * *"bars for the generators should be able to be 4 or 8 only, not 2 … every new
 * generation should generate 4/8 bars only and you should be able to see all 8
 * bars."* Two bars has no room for the fills and turnarounds the models author,
 * so generating at it made every artist sound the same.
 */
describe('the bar choices', () => {
  it('offers four and eight, and nothing shorter', () => {
    expect([...BAR_CHOICES]).toEqual([4, 8]);
  });

  it('still opens a project that was saved at a length no longer offered', () => {
    // ⛔ **The compatibility half, and it is the one that could bite somebody.**
    // Nothing here validates a restored value against `BAR_CHOICES` — the
    // engine clamps to `1..=MAX_BARS` on every path — so a session saved at two
    // bars must come back at two bars rather than being silently rounded up
    // into a pattern the producer did not arrange.
    useSession.getState().applyPreset({
      selectedId: 'trap',
      seed: '99',
      songSeed: '99',
      bars: 2,
      pins: null,
    });

    expect(useSession.getState().bars).toBe(2);
  });
});

describe('the drums seed a mirrored bass is told to copy', () => {
  // ⛔⛔ **A seed alone does not name a pattern — `(model, ctx, seed)` does.**
  // `Part::Bass` rebuilds its reference kit by re-running the drum generator at
  // this seed, so it only reproduces the clip on screen while the session that
  // built it still applies. Sending it regardless is the narrower form of the
  // very defect the field was added to close: the kick comes back on different
  // ticks and a `mirror_kick` bass lands on kicks nobody is playing.
  const drums: Pattern = { ...PATTERN, seed: '3141', songSeed: '7', bars: 4 };
  const now = { bars: 4, songSeed: '7', pins: NO_PINS, mood: null };

  it('is sent while the session still matches the drums on screen', () => {
    expect(mirrorableDrumsSeed(drums, now)).toBe('3141');
  });

  it('is withheld when there are no drums yet', () => {
    // Not a failure: there is no take to mirror, and the record's own canonical
    // kit is the right answer.
    expect(mirrorableDrumsSeed(undefined, now)).toBeNull();
  });

  it('is withheld once the bar count has moved under them', () => {
    // Generate drums at 4 bars, drag the chip to 8, generate the bass: the kick
    // would be rebuilt across 8 bars and land on different ticks entirely.
    expect(mirrorableDrumsSeed(drums, { ...now, bars: 8 })).toBeNull();
  });

  it('is withheld when the drums belong to a different record', () => {
    expect(mirrorableDrumsSeed(drums, { ...now, songSeed: '99' })).toBeNull();
  });

  it('is withheld when the mood has changed', () => {
    // A mode is a partial override of the model, including its session block —
    // so it can retune the key and the tempo the kick was written against.
    expect(mirrorableDrumsSeed(drums, { ...now, mood: 'dark' })).toBeNull();
  });

  it('is withheld when a pin moves the grid the kick sits on', () => {
    expect(mirrorableDrumsSeed(drums, { ...now, pins: { ...NO_PINS, bpm: 90 } })).toBeNull();
    expect(
      mirrorableDrumsSeed(drums, { ...now, pins: { ...NO_PINS, timeSigDen: 8 } }),
    ).toBeNull();
  });

  it('is sent when a pin merely agrees with what the drums already are', () => {
    // Pinning the tempo the drums were built at changes nothing about the kick,
    // so withholding there would give up the fix for no reason.
    expect(mirrorableDrumsSeed(drums, { ...now, pins: { ...NO_PINS, bpm: 140 } })).toBe('3141');
  });
});

/**
 * Switching a generation between four and eight bars (2026-08-11).
 *
 * ⛔⛔ **Mike:** *"if you already have a generation and you switch to 8 bars,
 * then it should copy the first 4 bars to the second 4 bars"* — and *"back to 4
 * bars again so the chords/melodies, etc. should double or go back to 4 bars."*
 * Without the copy, switching up gave four bars of music and four of silence,
 * which reads as the switch having half-broken the pattern.
 */
describe('the bar switch', () => {
  const clip = (): Pattern => ({
    id: 'p',
    part: 'drums',
    artistId: 'trap',
    seed: '1',
    songSeed: '1',
    bars: 4,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'natural_minor',
    ppq: 960,
    lanes: [
      {
        lane: 'kick',
        notes: [
          { startTick: 0, lenTicks: 120, pitch: 36, vel: 100 },
          { startTick: 960, lenTicks: 120, pitch: 36, vel: 100 },
        ],
      },
    ],
  });

  beforeEach(() => {
    useSession.setState({ bars: 4, patterns: { drums: clip() } });
  });

  it('repeats the first four bars into the second when it grows', () => {
    useSession.getState().setBars(8);
    const drums = useSession.getState().patterns.drums!;

    expect(drums.bars).toBe(8);
    // ⚠ One bar of 4/4 at 960 PPQ is 3840 ticks, so four bars is 15360.
    expect(drums.lanes[0].notes.map((n) => n.startTick)).toEqual([0, 960, 15360, 16320]);
  });

  it('drops what is past the end when it shrinks back', () => {
    // ⚠ The honest inverse. Notes left past the new boundary are in the file and
    // inaudible — the failure `does not carry a locked lane past the end of a
    // shorter clip` above records, arriving through the switch instead.
    useSession.getState().setBars(8);
    useSession.getState().setBars(4);
    const drums = useSession.getState().patterns.drums!;

    expect(drums.bars).toBe(4);
    expect(drums.lanes[0].notes.map((n) => n.startTick)).toEqual([0, 960]);
  });

  it('leaves the notes alone when the count has not moved', () => {
    // ⚠ Pressing the button you are already on must not double anything.
    useSession.getState().setBars(4);
    expect(useSession.getState().patterns.drums!.lanes[0].notes).toHaveLength(2);
  });

  it('tiles a clip that is not the store’s own length, rather than offsetting it', () => {
    // ⛔⛔ **An imported `.mid` keeps the file's bar count** — `openClip` stores
    // it verbatim — so the clip and the store disagree, and doubling "by the
    // store's 4 bars" put the copy at bar 5 of a two-bar clip and then rewrote
    // its length to 8. Four bars of silence in the middle of a clip that was
    // supposed to have been filled.
    const short = { ...clip(), bars: 2 };
    useSession.setState({ bars: 4, patterns: { drums: short } });

    useSession.getState().setBars(8);
    const drums = useSession.getState().patterns.drums!;

    expect(drums.bars).toBe(8);
    // Two bars is 7680 ticks, so eight bars is four passes of the same two hits.
    expect(drums.lanes[0].notes.map((n) => n.startTick)).toEqual([
      0, 960, 7680, 8640, 15360, 16320, 23040, 24000,
    ]);
  });

  it('trims a clip longer than the new length instead of copying past its end', () => {
    // ⚠ The mirror of the case above: a sixteen-bar import copied to bar 17 is
    // invisible in the roll and still in `patterns` for export.
    const long = { ...clip(), bars: 16 };
    useSession.setState({ bars: 4, patterns: { drums: long } });

    useSession.getState().setBars(8);
    const drums = useSession.getState().patterns.drums!;

    expect(drums.bars).toBe(8);
    expect(drums.lanes[0].notes.map((n) => n.startTick)).toEqual([0, 960]);
  });

  it('latches `edited`, or the doubling is not in the saved project', () => {
    // ⛔⛔ `editPattern` is the only other writer of `patterns` and it is what
    // sets this. Without it `send()` never stores the clip, so a project
    // reopened after the switch regenerates eight bars from the seed — different
    // material from the two passes the producer was shown and saved.
    expect(useSession.getState().edited).toBe(false);
    useSession.getState().setBars(8);

    expect(useSession.getState().edited).toBe(true);
    expect(useSession.getState().editedParts).toContain('drums');
  });

  it('does not mark an empty session edited', () => {
    // ⚠ Pressing 8 with nothing generated has nothing to save.
    useSession.setState({ bars: 4, patterns: {}, edited: false, editedParts: [] });
    useSession.getState().setBars(8);

    expect(useSession.getState().edited).toBe(false);
  });
});

/**
 * The genre an artist is generated in (TASK-158C).
 *
 * ⛔ What these are for is that the pin has to reach **two** places: the request
 * `generate` sends, and the defaults the chips are drawn from. A pin that
 * reached only the first would generate boom-bap while the tempo chip went on
 * showing the artist's own — which is the readout-that-lies failure the whole
 * task is closing, arriving through the fix.
 */
describe('the base genre', () => {
  beforeEach(() => {
    useSession.setState({ selectedId: 'trap', base: null, patterns: {} });
    invoke.mockImplementation((command: string) =>
      command === 'generate_pattern'
        ? Promise.resolve({
            id: 'x',
            artistId: 'trap',
            seed: '1',
            songSeed: '1',
            bars: 4,
            bpm: 140,
            timeSigNum: 4,
            timeSigDen: 4,
            keyRoot: 0,
            scale: 'naturalMinor',
            part: 'drums',
            lanes: [],
            ppq: 960,
            mood: null,
          })
        : Promise.resolve(null),
    );
  });

  const sentRequest = () =>
    (
      invoke.mock.calls.find(([command]) => command === 'generate_pattern')?.[1] as {
        request: { base: string | null };
      }
    ).request;

  it('travels with the generate request', async () => {
    useSession.setState({ base: 'boom-bap' });
    await useSession.getState().generate('drums');
    expect(sentRequest().base).toBe('boom-bap');
  });

  it('is null rather than absent when the artist keeps their own', async () => {
    // ⚠ The plugin reads `null` and absent the same way, and the page sends one
    // of them rather than omitting the key — an omitted key would make "their
    // own" indistinguishable from a payload that predates the field.
    await useSession.getState().generate('drums');
    expect(sentRequest()).toHaveProperty('base', null);
  });

  it('re-reads the defaults, because the base changes them', async () => {
    // ⛔ `bpm`, the tempo range and the mood names all come from the resolved
    // model, and the plugin now resolves it over this pin. Without the refetch
    // the tempo chip keeps showing the artist's own next to a beat that is
    // about to come out at the chosen genre's.
    invoke.mockClear();
    useSession.getState().setBase('boom-bap');
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('session_defaults', { styleId: 'trap' }),
    );
  });

  it('and so does the mood, which changes exactly the same three readouts', async () => {
    // ⛔ The gap the 2026-08-14 handoff wrote down: the plugin has resolved the
    // pinned mood inside `session_defaults` since TASK-040V — trap is 140, its
    // `dark` mode 136 — so a mood pinned without this refetch left the tempo
    // chip naming 140 beside a beat about to come out at 136, and it caught up
    // only on the next artist change.
    invoke.mockClear();
    useSession.getState().setMood('dark');
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('session_defaults', { styleId: 'trap' }),
    );
  });

  it('is carried into the undo snapshot, so Ctrl+Z puts it back', async () => {
    // ⛔ `SAVED_FIELDS` drives both the project payload and the undo snapshot,
    // and `SAVED_FIELDS_MATCH_SNAPSHOT` is a *compile-time* check that the two
    // lists agree. What it cannot check is that the value actually travels —
    // a field in the list and missing from `snapshotOf`'s destructure would
    // save less than it undoes, which is the drift that list exists to prevent.
    const { useHistory } = await import('./history');
    useSession.getState().setBase('boom-bap');
    await vi.waitFor(() => expect(useHistory.getState().present?.state.base).toBe('boom-bap'));
  });
});

/**
 * What an import says about the parts and lanes it did NOT find (TASK-058H).
 *
 * ⛔⛔ **The failure this prevents is the readout-that-lies one.** Mike,
 * 2026-08-10: *"ensure that when you bring in a full song that it mutes whatever
 * lanes for drums that aren't being used or if there is no countermelody or no
 * bassline that it mutes them."* After an import that found drums and a melody
 * but no bass, the Bass tab sits armed and empty — which is indistinguishable
 * from a bass that generated silence — and the pad grid draws thirty-seven lanes
 * of which five sound.
 */
describe('an import switches off what it did not find', () => {
  /** A drum clip that plays exactly `lanes`. */
  const drumsOn = (lanes: Lane[]): SplitPart => ({
    part: 'drums',
    reason: 'percussiveBand',
    notes: lanes.length,
    pattern: {
      ...PATTERN,
      part: 'drums',
      lanes: lanes.map((lane) => ({
        lane,
        notes: [
          {
            startTick: 0,
            lenTicks: 240,
            pitch: 36,
            vel: 100,
            modelVel: null,
            slideToPitch: null,
            articulation: null,
            reversed: false,
          },
        ],
      })),
    },
  });

  const melody: SplitPart = {
    part: 'melody',
    reason: 'melodicBand',
    notes: 1,
    pattern: { ...PATTERN, part: 'melody', lanes: [] },
  };

  beforeEach(() => {
    useSession.setState({ patterns: {}, mutedLanes: [], editedParts: [], edited: false });
    useUi.setState({ partsOff: [] });
  });

  it('a generator the split produced nothing for is switched off', () => {
    useSession.getState().importSplit([drumsOn(['kick', 'snare']), melody]);
    // ⚠ Bass, chords and counter were not in the split. Left armed they would be
    // three tabs a producer presses Play on and hears nothing from.
    expect(useUi.getState().partsOff.sort()).toEqual(['bass', 'chords', 'counter']);
    expect(useUi.getState().partsOff).not.toContain('drums');
  });

  it('a part the producer generated earlier is switched off too, and not deleted', () => {
    // ⛔⛔ **This is the half that was wrong first.** Sparing an earlier
    // generation sounds protective and is the failure pointing the other way: a
    // bassline made five minutes ago, still sounding under a song that has none,
    // makes the arrangement play something the imported file does not contain.
    useSession.setState({ patterns: { chords: { ...PATTERN, part: 'chords' } } });
    useSession.getState().importSplit([drumsOn(['kick'])]);
    expect(useUi.getState().partsOff).toContain('chords');
    // ⚠ Switched off, never deleted — the clip is untouched and one click back.
    expect(useSession.getState().patterns.chords).toBeDefined();
  });

  it('a drum lane with no hits in it is muted', () => {
    useSession.getState().importSplit([drumsOn(['kick', 'snare', 'closedHat'])]);
    const { mutedLanes } = useSession.getState();
    for (const played of ['kick', 'snare', 'closedHat']) {
      expect(mutedLanes, `${played} was played and must sound`).not.toContain(played);
    }
    expect(mutedLanes).toContain('openHat');
    expect(mutedLanes).toContain('clap');
  });

  it('a lane the previous import muted comes back when this one plays it', () => {
    // ⛔ **The mutes are REPLACED, not added to.** Carrying one forward would
    // leave a lane silent that this record does use, with a dot on the grid
    // saying otherwise.
    useSession.getState().importSplit([drumsOn(['kick'])]);
    expect(useSession.getState().mutedLanes).toContain('snare');
    useSession.getState().importSplit([drumsOn(['kick', 'snare'])]);
    expect(useSession.getState().mutedLanes).not.toContain('snare');
  });

  it('a producer’s own mute on a melodic lane survives an import', () => {
    // ⚠ Only the drum half is a statement the import gets to make — `melody` is
    // one lane of one part, so "the lanes this part did not use" means nothing
    // there.
    useSession.setState({ mutedLanes: ['melody'] });
    useSession.getState().importSplit([drumsOn(['kick'])]);
    expect(useSession.getState().mutedLanes).toContain('melody');
  });

  it('an import with no drums in it does not silence the producer’s kit', () => {
    // ⛔ Dropping a bass stem in must not mute thirty-seven lanes.
    useSession.setState({ mutedLanes: ['clap'] });
    useSession.getState().importSplit([melody]);
    expect(useSession.getState().mutedLanes).toEqual(['clap']);
  });

  it('the tab lands where the producer aimed, when they aimed at a part', () => {
    // ⚠ Dropping a sample on the Bass tab is an instruction about where to look,
    // even though what arrives is the whole split.
    useSession
      .getState()
      .importSplit([drumsOn(['kick', 'snare', 'closedHat']), melody], 'melody');
    expect(useUi.getState().activeTab).toBe('melody');
  });

  it('...and on the biggest part when they did not', () => {
    useSession.getState().importSplit([drumsOn(['kick', 'snare', 'closedHat']), melody]);
    expect(useUi.getState().activeTab).toBe('drums');
  });

  it('aiming at a part the split did not produce falls back rather than opening nothing', () => {
    useSession.getState().importSplit([drumsOn(['kick', 'snare']), melody], 'chords');
    expect(useUi.getState().activeTab).toBe('drums');
  });
});
