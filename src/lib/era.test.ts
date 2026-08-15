import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { DECADES, matchesEras, overlaps, parseEra, type Decade } from './era';

/**
 * The era filter's parser (TASK-158G).
 *
 * ⛔⛔ **The roadmap costed this task at normalizing the `era` field**, on the
 * reading that it holds "37 distinct free-text strings with no shared shape …
 * prose". Measured against the shipped dataset that is not what is there: 400
 * models, **every one** with an era, and **every one** parseable — 234 of the
 * 235 distinct strings are already regular, and the single irregular one is
 * `late 1990s–present`. So the cost is a parser that accepts two dash
 * characters and one qualifier, not a rewrite of 400 files.
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
  const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

  const eras = (): { model: string; era: string }[] => {
    const out: { model: string; era: string }[] = [];
    for (const dir of ['data/genres', 'data/artists']) {
      for (const file of readdirSync(join(root, dir)).filter((f) => f.endsWith('.json'))) {
        const model = JSON.parse(readFileSync(join(root, dir, file), 'utf8')) as {
          era?: unknown;
        };
        if (typeof model.era === 'string') out.push({ model: file, era: model.era });
      }
    }
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

  it('has every decade populated, so no pill is a dead control', () => {
    const all = eras();
    expect(all.length).toBeGreaterThan(100);
    for (const decade of DECADES) {
      const under = all.filter(({ era }) => matchesEras(era, [decade as Decade]));
      expect(under.length, `nobody is under the ${decade}s`).toBeGreaterThan(20);
    }
  });
});
