import { describe, expect, it } from 'vitest';
import { crossFilter } from './cross-filter';
import type { RosterEntry } from './ipc-types';

const model = (
  id: string,
  type: RosterEntry['type'],
  relatedGenres: string[] = [],
): RosterEntry => ({
  id,
  name: id,
  aliases: [],
  type,
  tier: null,
  genres: [],
  relatedGenres,
  era: null,
});

// Two artists over three genres, shaped like the shipped dataset: one artist in
// a single genre, one in several, and a genre nobody works in.
const roster: RosterEntry[] = [
  model('trap', 'genre'),
  model('plugg', 'genre'),
  model('phonk', 'genre'),
  model('drake', 'artist', ['trap']),
  model('summrs', 'artist', ['plugg', 'trap']),
];

describe('crossFilter', () => {
  it('narrows nothing when there is no selection', () => {
    const { artists, genres, filteredBy } = crossFilter(roster, null);

    expect(artists.map((a) => a.id)).toEqual(['drake', 'summrs']);
    expect(genres.map((g) => g.id)).toEqual(['trap', 'plugg', 'phonk']);
    expect(filteredBy).toBeNull();
  });

  it('shows only the artists in a picked genre', () => {
    const { artists, genres, filteredBy } = crossFilter(roster, 'trap');

    expect(artists.map((a) => a.id)).toEqual(['drake', 'summrs']);
    expect(genres.map((g) => g.id)).toEqual(['trap', 'plugg', 'phonk']);
    expect(filteredBy?.id).toBe('trap');
  });

  it('shows only the genres a picked artist works in, in authored order', () => {
    // Authored order, not roster order: `plugg` is summrs' own `extends`
    // parent and leads, though `trap` comes first in the roster.
    const { artists, genres, filteredBy } = crossFilter(roster, 'summrs');

    expect(genres.map((g) => g.id)).toEqual(['plugg', 'trap']);
    expect(artists.map((a) => a.id)).toEqual(['drake', 'summrs']);
    expect(filteredBy?.id).toBe('summrs');
  });

  it('reports a genre nobody works in as empty rather than as unfiltered', () => {
    // The artists' lists are the whole relation, so this is a real answer and
    // not a missing one — the rail says so in words.
    const { artists, filteredBy } = crossFilter(roster, 'phonk');

    expect(artists).toEqual([]);
    expect(filteredBy?.id).toBe('phonk');
  });

  it('narrows nothing for an artist nobody has curated', () => {
    // No list is "not recorded", which is not the claim that they work in no
    // genres — so it must not present as a genre list of none.
    const withUncurated = [...roster, model('nobody', 'artist')];
    const { genres, filteredBy } = crossFilter(withUncurated, 'nobody');

    expect(genres.map((g) => g.id)).toEqual(['trap', 'plugg', 'phonk']);
    expect(filteredBy).toBeNull();
  });

  it('narrows nothing when no artist in the dataset declares a genre', () => {
    // ⛔ Otherwise every genre empties the roster on an uncurated community
    // dataset, and the feature reads as the roster being broken.
    const uncurated = [model('trap', 'genre'), model('drake', 'artist')];

    expect(crossFilter(uncurated, 'trap').artists.map((a) => a.id)).toEqual(['drake']);
    expect(crossFilter(uncurated, 'trap').filteredBy).toBeNull();
  });

  it('ignores a selection that is not in the roster', () => {
    expect(crossFilter(roster, 'gone').filteredBy).toBeNull();
  });
});
