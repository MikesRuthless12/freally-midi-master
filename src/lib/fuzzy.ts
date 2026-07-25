/**
 * Roster search (FR-009, PRD § 3 Indexes).
 *
 * Runs in the frontend against the roster loaded once at startup, so a
 * keystroke costs no IPC round trip and no disk read — the budget is 50 ms for
 * a thousand entries and this is nowhere near it.
 *
 * The scoring is banded rather than continuous, because the ordering people
 * expect is categorical: anything the query *starts* is better than anything
 * that merely contains it, and both beat a typo correction. Within a band the
 * shorter name wins, so "future" the artist beats a genre that happens to
 * mention it, and a flagship breaks the tie after that.
 */

import type { RosterEntry } from './ipc-types';

/** Ranked bands. The gaps are wide so no in-band bonus can cross one. */
const BAND = {
  exact: 10_000,
  prefix: 8_000,
  wordStart: 6_000,
  substring: 4_000,
  subsequence: 2_000,
  typo: 1_000,
  none: 0,
} as const;

/** A hit on the name itself beats the same hit on an alias or a genre. */
const FIELD_WEIGHT = { name: 300, alias: 200, id: 150, genre: 50 } as const;

/** Flagships break ties — they are the roster the product is demonstrated with. */
const TIER_BONUS = { flagship: 40, standard: 20, inherited: 0 } as const;

/** How far a typo may be and still match. PRD § 3 says 2. */
const MAX_EDITS = 2;

/**
 * Lowercase, strip diacritics, and collapse punctuation to spaces.
 *
 * "Pierre Bourne" has to be findable as "pierre-bourne" and "pierrebourne",
 * and a user typing "beyonce" must find "Beyoncé". Normalising both sides is
 * the only way that holds for every language the roster might name.
 */
export function normalize(text: string): string {
  return text
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim();
}

/** Damerau-Levenshtein, bailing out once the distance passes `max`. */
export function editDistance(a: string, b: string, max = MAX_EDITS): number {
  if (a === b) return 0;
  if (Math.abs(a.length - b.length) > max) return max + 1;

  let previous2: number[] = [];
  let previous: number[] = Array.from({ length: b.length + 1 }, (_, i) => i);

  for (let i = 1; i <= a.length; i += 1) {
    const current: number[] = [i];
    let best = i;
    for (let j = 1; j <= b.length; j += 1) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      let value = Math.min(current[j - 1] + 1, previous[j] + 1, previous[j - 1] + cost);
      // The transposition that makes this Damerau rather than Levenshtein:
      // "drkae" for "drake" is one slip of the fingers, not two edits.
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) {
        value = Math.min(value, previous2[j - 2] + 1);
      }
      current[j] = value;
      best = Math.min(best, value);
    }
    // Every path through this row is already too expensive.
    if (best > max) return max + 1;
    previous2 = previous;
    previous = current;
  }

  return previous[b.length];
}

/** Whether every character of `query` appears in `text`, in order. */
function isSubsequence(query: string, text: string): boolean {
  let index = 0;
  for (const char of text) {
    if (char === query[index]) index += 1;
    if (index === query.length) return true;
  }
  return query.length === 0;
}

/** The band a single field earns against the query. */
function bandFor(query: string, field: string): number {
  if (!field) return BAND.none;
  if (field === query) return BAND.exact;
  if (field.startsWith(query)) return BAND.prefix;
  if (field.split(' ').some((word) => word.startsWith(query))) return BAND.wordStart;
  if (field.includes(query)) return BAND.substring;
  if (isSubsequence(query, field)) return BAND.subsequence;

  // Typos are compared word by word as well as whole: "drakee" should find
  // "drake", and "trvis scott" should find "travis scott" on the first word.
  if (editDistance(query, field) <= MAX_EDITS) return BAND.typo;
  if (field.split(' ').some((word) => editDistance(query, word) <= MAX_EDITS)) return BAND.typo;
  return BAND.none;
}

/**
 * Score one entry. 0 means no match at all.
 *
 * Exported for the tests, which assert the ordering rules directly rather than
 * only through the sorted output — a ranking bug is much easier to read as two
 * scores than as a reordered list.
 */
export function scoreEntry(query: string, entry: RosterEntry): number {
  const normalized = normalize(query);
  if (!normalized) return BAND.none;

  const candidates: [number, string][] = [
    [FIELD_WEIGHT.name, normalize(entry.name)],
    [FIELD_WEIGHT.id, normalize(entry.id)],
    ...entry.aliases.map((alias): [number, string] => [FIELD_WEIGHT.alias, normalize(alias)]),
    ...entry.genres.map((genre): [number, string] => [FIELD_WEIGHT.genre, normalize(genre)]),
  ];

  let best: number = BAND.none;
  for (const [weight, field] of candidates) {
    const band = bandFor(normalized, field);
    if (band === BAND.none) continue;

    // Shorter fields win inside a band: a query that is most of the name is a
    // better answer than one that is a fragment of a longer one. Capped so it
    // can never outweigh the field weight it sits beneath.
    const brevity = Math.max(0, 40 - field.length);
    const total = band + weight + brevity;
    if (total > best) best = total;
  }

  if (best === BAND.none) return BAND.none;
  return best + (entry.tier ? TIER_BONUS[entry.tier] : 0);
}

/**
 * The best matches for a query, most relevant first.
 *
 * `limit` is 8 because that is what the autosuggest shows (FR-009); asking for
 * more is what the roster list does.
 */
export function search(query: string, entries: RosterEntry[], limit = 8): RosterEntry[] {
  const scored: { entry: RosterEntry; score: number }[] = [];
  for (const entry of entries) {
    const score = scoreEntry(query, entry);
    if (score > BAND.none) scored.push({ entry, score });
  }

  scored.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    // A stable, meaningful tie-break so the same query always lists the same
    // order — otherwise the dropdown reshuffles between keystrokes.
    return a.entry.name.localeCompare(b.entry.name);
  });

  return scored.slice(0, limit).map((s) => s.entry);
}
