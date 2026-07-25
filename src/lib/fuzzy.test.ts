import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { editDistance, normalize, scoreEntry, search } from './fuzzy';
import type { RosterEntry } from './ipc-types';

/**
 * The real roster, read from `data/` rather than mocked.
 *
 * FR-009's acceptance criteria name actual artists ("osa" → OsamaSon), so a
 * fixture would be testing the fixture. This reads what ships.
 */
function shippedRoster(): RosterEntry[] {
  const root = join(__dirname, '..', '..', 'data');
  const entries: RosterEntry[] = [];

  for (const dir of ['genres', 'artists']) {
    let files: string[];
    try {
      files = readdirSync(join(root, dir));
    } catch {
      continue;
    }
    for (const file of files) {
      if (!file.endsWith('.json') || file.startsWith('_')) continue;
      const model = JSON.parse(readFileSync(join(root, dir, file), 'utf8')) as {
        id: string;
        name: string;
        type: 'artist' | 'genre';
        aliases?: string[];
        tier?: RosterEntry['tier'];
        genres?: string[];
        era?: string;
      };
      entries.push({
        id: model.id,
        name: model.name,
        aliases: model.aliases ?? [],
        type: model.type,
        tier: model.tier ?? null,
        genres: model.genres ?? [],
        era: model.era ?? null,
      });
    }
  }
  return entries;
}

const roster = shippedRoster();
const top = (query: string) => search(query, roster)[0]?.id;

describe('normalize', () => {
  it('folds case, diacritics and punctuation together', () => {
    // All three spellings of the same name have to land on one string, or
    // "pierre bourne" cannot find `pierre-bourne`.
    expect(normalize('Pierre-Bourne')).toBe('pierre bourne');
    expect(normalize('Beyoncé')).toBe('beyonce');
    expect(normalize('  UK   Drill! ')).toBe('uk drill');
  });
});

describe('editDistance', () => {
  it('counts a transposition as one slip, not two', () => {
    // Damerau, not plain Levenshtein: swapped letters are the commonest typo
    // there is, and charging two edits for one puts it past the threshold.
    expect(editDistance('drkae', 'drake')).toBe(1);
    expect(editDistance('drakee', 'drake')).toBe(1);
    expect(editDistance('drake', 'drake')).toBe(0);
  });

  it('gives up rather than computing a distance nobody will use', () => {
    // The bail-out is what keeps the search inside its budget; it must report
    // "further than max" rather than a wrong small number.
    expect(editDistance('drake', 'metro boomin')).toBeGreaterThan(2);
  });
});

describe('search', () => {
  it('finds the artists FR-009 names', () => {
    expect(top('osa')).toBe('osamason');
    expect(top('metro')).toBe('metro-boomin');
    expect(top('pierre')).toBe('pierre-bourne');
    expect(top('travis')).toBe('travis-scott');
  });

  it('lets an artist beat the English word they share a name with', () => {
    // "future" is a word every model could plausibly mention; the artist is
    // what someone typing it wants.
    expect(top('future')).toBe('future');
  });

  it('corrects a typo', () => {
    expect(top('drakee')).toBe('drake');
    expect(top('metroo')).toBe('metro-boomin');
  });

  it('finds a genre by its own name', () => {
    expect(top('uk')).toBe('uk-drill');
    expect(top('boom bap')).toBe('boom-bap');
  });

  it('finds an artist by an alias nobody would guess the id from', () => {
    // The whole point of aliases: `drake` is not spelled "drizzy" anywhere in
    // the id or the name.
    expect(top('drizzy')).toBe('drake');
    expect(top('ovo')).toBe('drake');
  });

  it('prefers a prefix to a substring', () => {
    const prefix = scoreEntry('trap', {
      ...roster[0],
      name: 'Trap Soul',
      aliases: [],
      genres: [],
    });
    const substring = scoreEntry('trap', {
      ...roster[0],
      name: 'Neo Trap Revival',
      aliases: [],
      genres: [],
    });
    expect(prefix).toBeGreaterThan(substring);
  });

  it('breaks a tie toward the flagship', () => {
    const base = { ...roster[0], name: 'Same Name', aliases: [], genres: [] };
    const flagship = scoreEntry('same', { ...base, tier: 'flagship' });
    const standard = scoreEntry('same', { ...base, tier: 'standard' });
    expect(flagship).toBeGreaterThan(standard);
  });

  it('returns nothing for a query that matches nothing', () => {
    expect(search('qqqzzzxxx', roster)).toEqual([]);
    expect(search('', roster)).toEqual([]);
    expect(search('   ', roster)).toEqual([]);
  });

  it('shows at most eight suggestions', () => {
    // FR-009: the dropdown renders ≤ 8. A single letter matches most of the
    // roster by subsequence, which is exactly the case that needs capping.
    expect(search('a', roster).length).toBeLessThanOrEqual(8);
  });

  it('orders identically for the same query every time', () => {
    // A dropdown that reshuffles between identical keystrokes is unusable.
    const once = search('tr', roster).map((e) => e.id);
    const twice = search('tr', roster).map((e) => e.id);
    expect(once).toEqual(twice);
  });

  it('stays inside the keystroke budget on a roster far larger than ours', () => {
    // FR-009 budgets 50 ms over 1,000 entries. Ours is 26, so this synthesizes
    // the size the requirement is actually written against.
    const many: RosterEntry[] = Array.from({ length: 1000 }, (_, i) => ({
      id: `artist-${i}`,
      name: `Artist Number ${i}`,
      aliases: [`a${i}`, `alias ${i}`],
      type: 'artist',
      tier: 'standard',
      genres: ['trap', 'drill'],
      era: null,
    }));

    const started = performance.now();
    for (const query of ['a', 'art', 'artist 9', 'nmbr', 'artst']) search(query, many);
    const elapsed = performance.now() - started;

    expect(elapsed).toBeLessThan(50 * 5);
  });

  it('has a roster to search at all', () => {
    // Every assertion above passes vacuously if the fixture loaded nothing.
    expect(roster.length).toBeGreaterThanOrEqual(20);
    expect(roster.some((e) => e.type === 'artist')).toBe(true);
    expect(roster.some((e) => e.type === 'genre')).toBe(true);
  });
});
