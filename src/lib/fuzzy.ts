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

/**
 * How far a subsequence may spread before it stops being evidence of anything.
 *
 * ⛔⛔ **A subsequence with no bound outranks a real typo, and the roster proved
 * it.** `drakee` is genuinely a subsequence of **d**imit**r**i veg**a**s
 * li**k**e mik**e** — six characters picked out of twenty-three — so Dimitri
 * Vegas & Like Mike scored 2337 against Drake's 1375 and *"drakee"* stopped
 * finding Drake. The subsequence band is 2,000 and the typo band 1,000, so any
 * long enough name beats a one-character correction on a short one.
 *
 * ⚠ **This gets worse as the roster grows, which is why it is a threshold and
 * not a special case.** Volume 2 took `data/` from 601 models to 1,300, and the
 * longer a name is the more short queries it swallows. Nothing was wrong with
 * the Dimitri Vegas entry.
 *
 * ⚠ **2× rather than tighter**, because the band's own purpose is a query that
 * is *most of* a name typed without its spaces: `pierrebourne` spans 13 of
 * `pierre bourne` for a 12-character query (1.08×) and `travisscott` 12 of
 * `travis scott` (1.09×). Both still match; `drakee` at 3.8× does not.
 */
const MAX_SUBSEQUENCE_SPREAD = 2;

/**
 * The span `query` occupies inside `text` as a subsequence, or -1 for no match.
 *
 * ⚠ **Forward to a match, then backward to tighten it.** A plain left-to-right
 * greedy scan takes the *first* candidate for each character, which overstates
 * the span — `ab` in `a x a b` reads as 4 when the real window is 2 — and would
 * then reject dense matches. Walking back from the completion point to the
 * latest possible start gives the minimal window ending there, in one extra
 * pass rather than the O(n·m) of trying every start.
 */
function subsequenceSpan(query: string, text: string): number {
  if (query.length === 0) return 0;
  let index = 0;
  for (let at = 0; at < text.length; at += 1) {
    if (text[at] !== query[index]) continue;
    index += 1;
    if (index < query.length) continue;

    let back = query.length - 1;
    for (let from = at; from >= 0; from -= 1) {
      if (text[from] !== query[back]) continue;
      back -= 1;
      if (back < 0) return at - from + 1;
    }
  }
  return -1;
}

/**
 * The band a single field earns against the query.
 *
 * ⚠ **`words` is passed in rather than split here.** It was `field.split(' ')`
 * *twice* in this function, which is two array allocations per field per
 * keystroke — around ten thousand of them to answer one character typed into the
 * roster box at the size `data/` ships. It is a property of the field, so it is
 * cached with the field; see `fieldsOf`.
 */
function bandFor(query: string, field: string, words: string[]): number {
  if (!field) return BAND.none;
  if (field === query) return BAND.exact;
  if (field.startsWith(query)) return BAND.prefix;
  if (words.some((word) => word.startsWith(query))) return BAND.wordStart;
  if (field.includes(query)) return BAND.substring;
  // ⚠ Bounded by how far it spreads — see `MAX_SUBSEQUENCE_SPREAD`. An
  // unbounded subsequence outranks a genuine typo on a shorter name.
  const span = subsequenceSpan(query, field);
  if (span >= 0 && span <= query.length * MAX_SUBSEQUENCE_SPREAD) return BAND.subsequence;

  // Typos are compared word by word as well as whole: "drakee" should find
  // "drake", and "trvis scott" should find "travis scott" on the first word.
  if (editDistance(query, field) <= MAX_EDITS) return BAND.typo;
  if (words.some((word) => editDistance(query, word) <= MAX_EDITS)) return BAND.typo;
  return BAND.none;
}

/**
 * One entry's searchable fields, normalized, weighted, and kept.
 *
 * ⛔⛔ **Because this was re-derived on every keystroke, for every entry**
 * (TASK-158E). A roster entry carries a name, an id, its aliases and its genres
 * — call it nine strings — and each one was run through `normalize` (a Unicode
 * NFD decomposition and two regex passes) every time a character was typed. At
 * the **590 entries `data/` ships today** that is over five thousand
 * normalizations per keystroke, for strings that have not changed since the
 * roster was read at startup. Measured at 5.5 ms a keystroke before this, on top
 * of React rebuilding up to 534 menu rows in the same frame.
 *
 * ⚠ **A `WeakMap`, so nothing is pinned in memory.** The roster is replaced
 * wholesale when a user model is saved, and a `Map` keyed on entries would hold
 * every superseded copy alive for the life of the page.
 *
 * ⚠ **Keyed on the entry object, not on its id.** Two objects with one id is
 * exactly what a save-and-reload produces, and the *new* one is the one whose
 * fields must be read.
 */
type Field = { weight: number; text: string; words: string[] };

const FIELDS = new WeakMap<RosterEntry, Field[]>();

function fieldsOf(entry: RosterEntry): Field[] {
  const held = FIELDS.get(entry);
  if (held !== undefined) return held;
  const of = (weight: number, raw: string): Field => {
    const text = normalize(raw);
    return { weight, text, words: text.split(' ') };
  };
  const built: Field[] = [
    of(FIELD_WEIGHT.name, entry.name),
    of(FIELD_WEIGHT.id, entry.id),
    ...entry.aliases.map((alias) => of(FIELD_WEIGHT.alias, alias)),
    ...entry.genres.map((genre) => of(FIELD_WEIGHT.genre, genre)),
  ];
  FIELDS.set(entry, built);
  return built;
}

/**
 * Score one entry against a query that is **already normalized**.
 *
 * ⚠ Split out from [`scoreEntry`] so `search` can normalize the query once
 * rather than once per entry — the same string was being NFD-decomposed 590
 * times to answer one keystroke.
 */
function scoreNormalized(normalized: string, entry: RosterEntry): number {
  if (!normalized) return BAND.none;

  let best: number = BAND.none;
  for (const field of fieldsOf(entry)) {
    const band = bandFor(normalized, field.text, field.words);
    if (band === BAND.none) continue;

    // Shorter fields win inside a band: a query that is most of the name is a
    // better answer than one that is a fragment of a longer one. Capped so it
    // can never outweigh the field weight it sits beneath.
    const brevity = Math.max(0, 40 - field.text.length);
    const total = band + field.weight + brevity;
    if (total > best) best = total;
  }

  if (best === BAND.none) return BAND.none;
  return best + (entry.tier ? TIER_BONUS[entry.tier] : 0);
}

/**
 * Score one entry. 0 means no match at all.
 *
 * Exported for the tests, which assert the ordering rules directly rather than
 * only through the sorted output — a ranking bug is much easier to read as two
 * scores than as a reordered list.
 */
export function scoreEntry(query: string, entry: RosterEntry): number {
  return scoreNormalized(normalize(query), entry);
}

/**
 * The best matches for a query, most relevant first.
 *
 * `limit` is 8 because that is what the autosuggest shows (FR-009); asking for
 * more is what the roster list does.
 */
export function search(query: string, entries: RosterEntry[], limit = 8): RosterEntry[] {
  // ⛔ **Once, not once per entry.** See `fieldsOf` — this is the other half of
  // the same waste, and it is the cheaper half to fix.
  const normalized = normalize(query);
  const scored: { entry: RosterEntry; score: number }[] = [];
  for (const entry of entries) {
    const score = scoreNormalized(normalized, entry);
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
