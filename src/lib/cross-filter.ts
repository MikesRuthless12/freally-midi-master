import type { RosterEntry } from './ipc-types';

/**
 * The roster, narrowed to what the current selection works with.
 *
 * Picking a genre shows only the artists in it; picking an artist shows only
 * the genres that artist works in. Both directions come from one authored
 * field — `relatedGenres` on the artist — because only one of them is data:
 * the reverse is derived by inverting it here.
 *
 * ⛔ **Not derivable from `RosterEntry.genres`, and that is worth knowing before
 * anyone deletes this in favour of a one-line intersection.** That field is
 * free-text tags in a vocabulary of its own — `rap`, `drill`, `club` — which
 * name no model at all, and `rap` is on almost every model in the dataset, so
 * matching on tags relates nearly everything to nearly everything. `extends` is
 * no better in the other direction: it carries exactly one parent per artist,
 * so "the genres this artist works in" would always be a list of one.
 */
export type CrossFiltered = {
  artists: RosterEntry[];
  genres: RosterEntry[];
  /** The entry the narrowing came from, or `null` when nothing is narrowed. */
  filteredBy: RosterEntry | null;
};

export function crossFilter(roster: RosterEntry[], selectedId: string | null): CrossFiltered {
  // ⛔ **`!== 'genre'`, not `=== 'artist'`.** Producers became their own type on
  // 2026-08-12, and every filter here spelled as "the artists" would have begun
  // hiding 190 of the 534 names the day that variant landed — silently, because
  // a shorter list is not an error. The question this asks has always been "is
  // it a style a producer picks", never "is it specifically an artist".
  const artists = roster.filter((entry) => entry.type !== 'genre');
  const genres = roster.filter((entry) => entry.type === 'genre');
  const everything: CrossFiltered = { artists, genres, filteredBy: null };

  const selected = roster.find((entry) => entry.id === selectedId);
  if (!selected) return everything;

  // ⛔ A dataset nobody has curated must not filter at all. Every direction here
  // reads the artists' lists, so if no artist declares one, every genre would
  // narrow the roster to nobody — the feature would present as the roster being
  // broken, on exactly the community datasets least able to diagnose it.
  if (!artists.some((artist) => artist.relatedGenres.length > 0)) return everything;

  if (selected.type === 'artist') {
    // An artist with no list has not been curated, which is not the same claim
    // as working in no genres — so it narrows nothing rather than showing none.
    if (selected.relatedGenres.length === 0) return everything;

    // Authored order, not roster order: the first entry is the model's own
    // `extends` parent, so the genre it is actually built from leads.
    const byId = new Map(genres.map((genre) => [genre.id, genre]));
    return {
      artists,
      genres: selected.relatedGenres
        .map((id) => byId.get(id))
        .filter((genre): genre is RosterEntry => genre !== undefined),
      filteredBy: selected,
    };
  }

  // The other direction, and it *can* legitimately be empty: the artists' lists
  // are the whole of the relation, so a genre no artist claims has none. Eight
  // of the fifteen shipped genres are in that position, which is why the rail
  // says so in words rather than rendering an empty list.
  return {
    artists: artists.filter((artist) => artist.relatedGenres.includes(selected.id)),
    genres,
    filteredBy: selected,
  };
}
