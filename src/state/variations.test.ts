import { beforeEach, describe, expect, it } from 'vitest';

import type { Pattern } from '../lib/ipc-types';
import { counter, entryFor, madeAt, useVariations } from './variations';

/**
 * The variation history (TASK-045).
 *
 * ⛔ Three claims are worth testing and none of them is "it stores things":
 * that the log has **no cap**, that an entry records what was *used* rather
 * than what was pinned, and that generating after stepping back does not throw
 * the entries ahead away.
 */

const clip = (part: Pattern['part'], seed: string, bpm: number): Pattern => ({
  id: `p-${seed}`,
  part,
  artistId: 'trap',
  seed,
  songSeed: seed,
  bars: 4,
  bpm,
  timeSigNum: 4,
  timeSigDen: 4,
  keyRoot: 6,
  scale: 'natural_minor',
  ppq: 960,
  lanes: [],
});

const NO_PINS = {
  bpm: null,
  keyRoot: null,
  scale: null,
  swing: null,
  timeSigNum: null,
  timeSigDen: null,
};

describe('the variation history', () => {
  // ⚠ **The persisted half is cleared too, and `reset()` cannot do it.** That
  // empties the store; the takes live outside it — behind the plugin in the real
  // app, in the IPC mock here — and outliving a page load is the entire point of
  // them (TASK-045B). Without this the thousand-take cap test above leaks into
  // every case after it, which is exactly the property working correctly.
  beforeEach(async () => {
    useVariations.getState().reset();
    await useVariations.getState().clearHistory();
  });

  it('keeps every generation of the session, with no cap', () => {
    // ⛔ Mike's rescope: *"keep going sequentially through the seeds that you
    // have generated since the beginning of the app."* The old "last 20" cap
    // was there to bound memory and does not need to be — an entry is tens of
    // bytes, because the notes are not stored.
    for (let i = 0; i < 1_000; i += 1) {
      useVariations
        .getState()
        .record(
          entryFor(clip('drums', String(i), 140), { mood: null, base: null, pins: NO_PINS }, i),
        );
    }
    expect(counter('drums')).toEqual({ position: 1_000, total: 1_000 });
    expect(useVariations.getState().entries.drums?.[0].seed).toBe('0');
  });

  it('counts each part separately', () => {
    // Rerolling one lane advances that part and nothing else, so one global
    // number would claim the chords changed when they did not.
    useVariations
      .getState()
      .record(entryFor(clip('drums', '1', 140), { mood: null, base: null, pins: NO_PINS }, 1));
    useVariations
      .getState()
      .record(entryFor(clip('drums', '2', 140), { mood: null, base: null, pins: NO_PINS }, 2));
    useVariations
      .getState()
      .record(entryFor(clip('melody', '3', 140), { mood: null, base: null, pins: NO_PINS }, 3));

    expect(counter('drums')).toEqual({ position: 2, total: 2 });
    expect(counter('melody')).toEqual({ position: 1, total: 1 });
    expect(counter('bass')).toEqual({ position: 0, total: 0 });
  });

  it('records the tempo that was used, not the one that was pinned', () => {
    // ⛔ **The important half of the rescope.** A generation made while the DAW
    // sat at 92 was made at 92, and the pins may say nothing about that —
    // showing the pins would show blank. And tempo changes the notes, so an
    // entry that recorded only the seed would not reproduce its own beat.
    const entry = entryFor(
      clip('drums', '7', 92),
      { mood: 'dark', base: null, pins: NO_PINS },
      5,
    );
    expect(entry.bpm).toBe(92);
    expect(entry.pins.bpm).toBeNull();
    expect(entry.mood).toBe('dark');
    // The whole setup, so stepping back restores how you got there.
    expect(entry.artistId).toBe('trap');
    expect(entry.bars).toBe(4);
    expect(entry.scale).toBe('natural_minor');
  });

  it('steps without wrapping, and clamps at both ends', () => {
    for (const seed of ['1', '2', '3']) {
      useVariations
        .getState()
        .record(
          entryFor(
            clip('drums', seed, 140),
            { mood: null, base: null, pins: NO_PINS },
            Number(seed),
          ),
        );
    }
    expect(useVariations.getState().step('drums', -1)?.seed).toBe('2');
    expect(useVariations.getState().step('drums', -1)?.seed).toBe('1');
    // ⚠ Clamped, not wrapped: wrapping would take a producer stepping back
    // through a thousand takes to the newest one with nothing saying so.
    expect(useVariations.getState().step('drums', -1)).toBeNull();
    expect(counter('drums')).toEqual({ position: 1, total: 3 });
  });

  it('generating after stepping back keeps the entries ahead', () => {
    // ⛔ Mike: *"starts a new branch from there rather than silently discarding
    // the entries ahead."* Losing forward history is what would cost someone a
    // beat they liked, so the log is append-only and stepping back is browsing.
    for (const seed of ['1', '2', '3']) {
      useVariations
        .getState()
        .record(
          entryFor(
            clip('drums', seed, 140),
            { mood: null, base: null, pins: NO_PINS },
            Number(seed),
          ),
        );
    }
    useVariations.getState().step('drums', -2);
    useVariations
      .getState()
      .record(entryFor(clip('drums', '4', 140), { mood: null, base: null, pins: NO_PINS }, 4));

    expect(useVariations.getState().entries.drums?.map((e) => e.seed)).toEqual([
      '1',
      '2',
      '3',
      '4',
    ]);
    // ...and the producer is parked on the one they just made.
    expect(counter('drums')).toEqual({ position: 4, total: 4 });
  });

  it('keeps takes per part, so one seed cannot star two generators', () => {
    // ⛔ The drums and the melody of one record share a song seed. Keying the
    // kept set on the seed alone would mean starring a melody silently starred
    // the drums with it, and a training set nobody chose.
    const { record, keep, keptEntries } = useVariations.getState();
    record(entryFor(clip('melody', '7', 140), { mood: null, base: null, pins: NO_PINS }, 7));
    record(entryFor(clip('drums', '7', 140), { mood: null, base: null, pins: NO_PINS }, 7));

    keep('melody', '7', true);

    const kept = useVariations.getState().keptEntries();
    expect(kept).toHaveLength(1);
    expect(kept[0].part).toBe('melody');
    expect(keptEntries).toBeTypeOf('function');
  });

  it('unkeeping takes a take back out of the training set', () => {
    const { record, keep } = useVariations.getState();
    record(entryFor(clip('melody', '11', 140), { mood: null, base: null, pins: NO_PINS }, 11));

    keep('melody', '11', true);
    expect(useVariations.getState().keptEntries()).toHaveLength(1);

    keep('melody', '11', false);
    expect(useVariations.getState().keptEntries()).toHaveLength(0);
  });

  it('stepping back through the log cannot change what was kept', () => {
    // ⛔ Keeping is an opinion about a take; the log is a record of what
    // happened. A flag living inside the entry would be one careless `record`
    // away from browsing the history rewriting the training set.
    const { record, keep, step } = useVariations.getState();
    for (const seed of ['1', '2', '3']) {
      record(
        entryFor(
          clip('melody', seed, 140),
          { mood: null, base: null, pins: NO_PINS },
          Number(seed),
        ),
      );
    }
    keep('melody', '2', true);

    step('melody', -2);
    record(entryFor(clip('melody', '4', 140), { mood: null, base: null, pins: NO_PINS }, 4));

    const kept = useVariations.getState().keptEntries();
    expect(kept.map((entry) => entry.seed)).toEqual(['2']);
  });

  /**
   * The history that outlives the session (TASK-045B).
   *
   * ⛔⛔ Mike: *"if you have generated 20 just 'Trap' and 20 just 'Rage' and 40
   * just 'Drake' then it should persist … so that way you can go through the
   * actual history of all your generations and find what you like."*
   *
   * ⚠ **Grouped by style here, by part in `entries`.** They are two structures
   * on purpose: `entries` is what ◀/▶ walk and its key has to be the part,
   * because the counter is per part; this is what the panel browses and its key
   * has to be the style, because that is the grouping in Mike's own sentence.
   */
  it('sends every generation to the plugin to be kept', async () => {
    const { record } = useVariations.getState();
    record(entryFor(clip('drums', '1', 140), { mood: null, base: null, pins: NO_PINS }, 1));
    record(entryFor(clip('melody', '2', 140), { mood: null, base: null, pins: NO_PINS }, 2));
    // The write is fire-and-forget — see `record` — so the reply lands a
    // microtask later.
    await Promise.resolve();
    await Promise.resolve();

    // ⚠ Asserted through `loadHistory` rather than by spying on `invoke`: what
    // matters is that a take written by one session is readable by the next,
    // which is the round trip rather than the call.
    await useVariations.getState().loadHistory();
    const held = useVariations.getState().history.trap ?? [];
    expect(held.map((take) => `${take.part}:${take.seed}`)).toEqual(['drums:1', 'melody:2']);
  });

  it('does not grow the kept history when a take is recorded twice', async () => {
    // ⛔ **Recalling a take regenerates it**, and `generate` records every
    // pattern it lands — `session.ts` guards that with its `recalling` flag, but
    // a guard on the page is not a rule on disk. Idempotent on `(part, seed)`,
    // which is `keptKey`'s pairing and for the same reason: the drums and the
    // melody of one record share a song seed.
    const { record } = useVariations.getState();
    const take = entryFor(
      clip('drums', '9', 140),
      { mood: null, base: null, pins: NO_PINS },
      9,
    );
    record(take);
    record(take);
    await Promise.resolve();
    await Promise.resolve();

    await useVariations.getState().loadHistory();
    expect(useVariations.getState().history.trap ?? []).toHaveLength(1);
  });

  it('can be forgotten, because it is a record of what somebody has been making', async () => {
    useVariations
      .getState()
      .record(entryFor(clip('drums', '3', 140), { mood: null, base: null, pins: NO_PINS }, 3));
    await Promise.resolve();
    await Promise.resolve();

    await useVariations.getState().clearHistory();
    expect(useVariations.getState().history).toEqual({});
  });

  /**
   * Files kept to train on (TASK-040T).
   *
   * ⛔ **The one thing in this store that carries notes.** Everything else is a
   * seed, because the engine rebuilds a generation exactly — and nothing rebuilds
   * somebody else's `.mid`, so a kept file has to be the material itself.
   */
  describe('a file kept to train on', () => {
    const split = (path: string, parts: Pattern['part'][]) => ({
      path,
      patterns: parts.map((part, index) => clip(part, `${path}-${index}`, 140)),
    });

    it('is kept and dropped through the same call', () => {
      const file = split('/lib/riff.mid', ['melody', 'bass']);
      useVariations.getState().keepFile(file, true);
      expect(useVariations.getState().keptFilePatterns()).toHaveLength(2);

      useVariations.getState().keepFile(file, false);
      expect(useVariations.getState().keptFilePatterns()).toEqual([]);
    });

    it('counts once however many times it is kept', () => {
      // ⛔ A fit measures a distribution: the same eight bars counted three times
      // reports the producer's taste as three times more certain than it is.
      const file = split('/lib/riff.mid', ['melody', 'bass']);
      useVariations.getState().keepFile(file, true);
      useVariations.getState().keepFile(file, true);
      useVariations.getState().keepFile(file, true);
      expect(useVariations.getState().keptFilePatterns()).toHaveLength(2);
    });

    it('keeps two different files side by side', () => {
      useVariations.getState().keepFile(split('/lib/a.mid', ['melody']), true);
      useVariations.getState().keepFile(split('/lib/b.mid', ['drums', 'bass']), true);
      expect(useVariations.getState().keptFilePatterns()).toHaveLength(3);
    });

    it('is forgotten by reset, like the rest of the session', () => {
      useVariations.getState().keepFile(split('/lib/a.mid', ['melody']), true);
      useVariations.getState().reset();
      expect(useVariations.getState().keptFilePatterns()).toEqual([]);
    });

    it('is a separate set from the kept takes, which are seeds', () => {
      // Keeping a take must not touch the files, and the reverse: the two are
      // different kinds of thing and `trainFromKept` sends both.
      useVariations
        .getState()
        .record(
          entryFor(clip('drums', '9', 140), { mood: null, base: null, pins: NO_PINS }, 9),
        );
      useVariations.getState().keep('drums', '9', true);
      useVariations.getState().keepFile(split('/lib/a.mid', ['melody']), true);

      expect(useVariations.getState().keptEntries()).toHaveLength(1);
      expect(useVariations.getState().keptFilePatterns()).toHaveLength(1);
    });
  });

  it('writes the date the way it was asked for, through Intl', () => {
    // ⛔ Two formatters joined by " @", because `timeStyle` cannot be combined
    // with `timeZoneName`. A literal `dddd, MMMM D, YYYY` would be right in one
    // locale and wrong in the other seventeen — two of which are RTL.
    const at = Date.UTC(2026, 7, 13, 21, 54);
    const text = madeAt(at, 'en-US');
    expect(text).toMatch(/^[A-Za-z]+day, August \d{1,2}, 2026 @\d{1,2}:\d{2}\s?[AP]M/);
    expect(text).toContain(' @');
    // Nothing to show for a pattern saved before the field existed.
    expect(madeAt(0, 'en-US')).toBe('');
  });
});
