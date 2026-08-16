import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { DECADES, matchesEras, overlaps, parseEra } from './era';
import type { RosterSummary } from './ipc-types';

/**
 * The era filter's parser (TASK-158G).
 *
 * ⛔⛔ **The roadmap costed this task at normalizing the `era` field**, on the
 * reading that it holds "37 distinct free-text strings with no shared shape …
 * prose". Measured against the shipped dataset that is not what is there: **590
 * models** — 56 genres, 344 artists, 190 producers — **every one** with an era
 * and **every one** parseable. 234 of the 235 distinct strings are already
 * regular and the single irregular one is `late 1990s–present`. So the cost is
 * a parser that accepts two dash characters and one qualifier, not a rewrite of
 * 590 files.
 *
 * ▶ `the_whole_shipped_dataset_parses` is what keeps that true: it reads
 * `data/` rather than a fixture, so the day somebody authors a shape this
 * cannot read, this fails rather than the pill silently hiding them.
 */

describe('reading an era', () => {
  it('reads a plain span, in either dash', () => {
    expect(parseEra('1993-2005')).toEqual({ from: 1993, to: 2005 });
    // ⛔ The en dash ships too, and a parser that knew only the hyphen would
    // file the same era under two answers with nothing on screen saying why.
    expect(parseEra('1993–2005')).toEqual({ from: 1993, to: 2005 });
  });

  it('reads one year as one year, and a decade as ten', () => {
    expect(parseEra('1991')).toEqual({ from: 1991, to: 1991 });
    // ⚠ The `s` is the whole difference. Without it every decade-spelled entry
    // would be a single-year one, and `boom-bap` would stop matching the 2000s.
    expect(parseEra('1990s')).toEqual({ from: 1990, to: 1999 });
    expect(parseEra('1990s-2000s')).toEqual({ from: 1990, to: 2009 });
  });

  it('leaves "present" open rather than guessing this year', () => {
    expect(parseEra('2013–present')).toEqual({ from: 2013, to: Infinity });
    expect(parseEra('1990s–present')).toEqual({ from: 1990, to: Infinity });
  });

  it('drops a qualifier rather than inventing a precision for it', () => {
    // The one irregular string in the whole dataset. "late" as 1995 would be a
    // number the researcher did not write, and the filter works in decades.
    expect(parseEra('late 1990s–present')).toEqual({ from: 1990, to: Infinity });
  });

  it('answers null for something it cannot read, rather than a wrong span', () => {
    expect(parseEra(null)).toBeNull();
    expect(parseEra('the nineties')).toBeNull();
    expect(parseEra('')).toBeNull();
  });

  it('clamps a transposed span rather than producing one that matches nothing', () => {
    // ⛔ **The failure direction matters more than the value.** `2000s-1990s`
    // parses cleanly to an *empty* span, which overlaps no decade at all — so a
    // typo would hide a model from all four pills, where an unreadable era shows
    // under every one. The gate over `data/` proves parseability, not sanity.
    //
    // ⚠ **Collapsed to the start, not widened to cover both ends.** Reading it
    // as 1990–2009 would be inventing the intention behind a typo, which is the
    // same thing this module refuses to do with "late". A one-year span at the
    // year that was actually written is the smallest repair that leaves the
    // model reachable.
    expect(parseEra('2000s-1990s')).toEqual({ from: 2000, to: 2000 });
    expect(matchesEras('2000s-1990s', [2000])).toBe(true);
    expect(matchesEras('2000s-1990s', [1990])).toBe(false);
  });
});

describe('which decades a span belongs to', () => {
  it('puts a long span under every decade it touches, not one', () => {
    // ⛔ The property that makes this a filter rather than a sort. `boom-bap` is
    // 1990s–present; a sort would force it into one bucket and lie about three.
    const span = parseEra('1990s–present');
    expect(span).not.toBeNull();
    for (const decade of DECADES) {
      expect(overlaps(span as { from: number; to: number }, decade), `${decade}`).toBe(true);
    }
  });

  it('excludes a decade the span ends before and one it begins after', () => {
    const span = { from: 2003, to: 2008 };
    expect(overlaps(span, 2000)).toBe(true);
    expect(overlaps(span, 1990)).toBe(false);
    expect(overlaps(span, 2010)).toBe(false);
  });

  it('counts a span that only clips the edge of a decade', () => {
    // 1999–2001 is a 90s record and a 2000s one. Rounding it to whichever end
    // has more years in it would drop it from a decade it genuinely spans.
    expect(overlaps({ from: 1999, to: 2001 }, 1990)).toBe(true);
    expect(overlaps({ from: 1999, to: 2001 }, 2000)).toBe(true);
  });
});

describe('what the pills show', () => {
  it('shows everything when nothing is pressed', () => {
    // ⛔ Pills that hid the roster until one was pressed would make the list
    // look broken in the state it spends most of its life in.
    expect(matchesEras('1993-1995', [])).toBe(true);
    expect(matchesEras(null, [])).toBe(true);
  });

  it('shows a style whose era it cannot read, whatever is pressed', () => {
    // ⚠ In practice this is the producer's own style, authored in an editor
    // that never asks for an era. Hiding somebody's own work behind a filter
    // they were offered no way to satisfy is the worse failure by a distance.
    expect(matchesEras(null, [2020])).toBe(true);
    expect(matchesEras('the nineties', [2020])).toBe(true);
  });

  it('is a union across the pressed pills, not an intersection', () => {
    // Two pills means "either", the way a multi-select reads. An intersection
    // would make pressing a second pill *narrow* the list, which is backwards.
    expect(matchesEras('1994-1997', [1990, 2020])).toBe(true);
    expect(matchesEras('2021–present', [1990, 2020])).toBe(true);
    expect(matchesEras('2003-2006', [1990, 2020])).toBe(false);
  });
});

describe('the shipped dataset', () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'data');

  /**
   * ⛔⛔ **All THREE directories, and reading two is a mistake this repo has a
   * postmortem for.** The producers moved into a folder of their own on
   * 2026-08-12 and `lib/fuzzy.test.ts` records what a two-directory reader cost:
   * *"Metro Boomin, Southside and Pi'erre Bourne vanished from the fixture — and
   * the failure reads as a product defect, not a missing file … Third reader of
   * `data/artists` the split caught out. Keep all three."* This was the fourth,
   * and it repeated the mistake the comment was written to stop: 190 of the 590
   * shipped models are producers, so the gate below claimed to cover the roster
   * while measuring two thirds of it — and a producer with an unparseable era is
   * a name that vanishes from every pill with nothing failing.
   *
   * ⚠ Read once. Two `it` blocks below both want it, and the walk is ~200 ms.
   */
  let cached: { model: string; era: string }[] | null = null;
  const eras = (): { model: string; era: string }[] => {
    if (cached !== null) return cached;
    const out: { model: string; era: string }[] = [];
    for (const dir of ['genres', 'artists', 'producers']) {
      for (const file of readdirSync(join(root, dir))) {
        // `_defaults.json` and friends are internal bases, not roster entries.
        if (!file.endsWith('.json') || file.startsWith('_')) continue;
        const model = JSON.parse(readFileSync(join(root, dir, file), 'utf8')) as {
          era?: unknown;
        };
        if (typeof model.era === 'string') out.push({ model: file, era: model.era });
      }
    }
    cached = out;
    return out;
  };

  it('parses every era it ships, so no model is silently unfilterable', () => {
    // ⛔ **Read from `data/`, not from a fixture.** A fixture would go on
    // passing the day somebody authors a shape this cannot read — and the
    // symptom of that is a model quietly missing from every pill, which is
    // invisible in a list of four hundred names.
    const unreadable = eras()
      .filter(({ era }) => parseEra(era) === null)
      .map(({ model, era }) => `${model}: ${era}`);
    expect(unreadable).toEqual([]);
  });

  it('reads all three model directories, not the two the split caught out', () => {
    // ⛔ The gate above is only as wide as this walk. 190 of the 590 shipped
    // models are producers, and reading `genres` + `artists` alone measured two
    // thirds of the roster while claiming the whole of it.
    expect(eras().length).toBeGreaterThan(500);
  });

  it('has every decade populated, so no pill is a dead control', () => {
    const all = eras();
    for (const decade of DECADES) {
      const under = all.filter(({ era }) => matchesEras(era, [decade]));
      expect(under.length, `nobody is under the ${decade}s`).toBeGreaterThan(20);
    }
  });
});

describe('the mock roster', () => {
  it('has eras that all parse, because an unparseable one is a row no pill can touch', async () => {
    // ⛔⛔ **The fixture is what every era spec is measured against, and it
    // shipped a value the parser refuses.** `uk-drill` carried `2018-`, which
    // matches no shape in `data/` and which `parseEra` answers `null` for — so
    // by `matchesEras`' "show what you cannot read" rule that genre appeared
    // under all four pills at once, in the one fixture the filter is proven
    // with. Exactly the failure this repo has already recorded: *"a 4-row
    // explorer fixture and a fixed seed hid two real bugs behind a green
    // suite."*
    const { mockInvoke } = await import('./ipc-mock');
    const summary = await mockInvoke<RosterSummary>('roster_summary');
    const unreadable = summary.entries
      .filter((entry) => entry.era !== null && parseEra(entry.era) === null)
      .map((entry) => `${entry.id}: ${entry.era}`);
    expect(unreadable).toEqual([]);
  });
});
