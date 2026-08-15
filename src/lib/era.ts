/**
 * When a style was current, as a span rather than a bucket (TASK-158G).
 *
 * ⛔⛔ **Mike, 2026-08-10:** *"allow the end user to [filter] the list by what
 * genre/artist was out within those specific years instead of trying to search
 * through them all and not finding what you want and just randomly searching for
 * names through genres/artists/producers blindly."*
 *
 * ▶ **This is the answer to the half a combobox cannot solve.** Typing only
 * works when you can already name the thing; browsing is what you do when you
 * cannot, and "what was out when I was listening" is the one axis a producer
 * always knows. Era beats alphabet, tier and genre for that job because it is
 * the axis their own taste is organised on.
 *
 * ⛔ **A FILTER, not a sort, and the difference is this type.** A model is
 * current across a *span*: `boom-bap` is `1990s–present`, so it belongs under
 * all four decades at once. A sort would force it into one bucket and lie about
 * three. [`overlaps`] is therefore the whole comparison.
 */

/** The four decades the pills offer, as the year each begins. */
export type Decade = 1990 | 2000 | 2010 | 2020;

export const DECADES: readonly Decade[] = [1990, 2000, 2010, 2020];

/** The years a style was current. `to` is `Infinity` for "and still is". */
export type Span = { from: number; to: number };

/**
 * ⛔ **`Infinity` rather than this year, and it is not laziness.** "…–present"
 * has no end, and substituting the current year would make this module read a
 * clock — which would give every test that touches it a different answer on
 * 1 January, and would silently stop matching the newest decade the moment one
 * began. An open span overlaps every decade from its start onwards, which is
 * exactly what `1990s–present` means and exactly the answer the roadmap asks
 * for: boom bap is a correct result for a producer browsing the 2020s today.
 */
const STILL_CURRENT = Infinity;

/**
 * A qualifier some entries open with. Dropped rather than honoured.
 *
 * ⚠ **One entry in the whole dataset uses one** — `late 1990s–present`. Reading
 * "late" as 1995 would be inventing a precision the researcher did not write,
 * and the decade is the resolution this filter works at anyway, so it cannot
 * change an answer.
 */
const QUALIFIER = /^(?:late|early|mid)[\s-]+/;

/**
 * The years an `era` string names, or `null` if it names none.
 *
 * ⚠ **Two dash characters ship and both are accepted.** `2013–present` (en
 * dash) and `2013-present` (hyphen) are both in `data/`, and a parser that knew
 * only one would file the same era under two answers with nothing on screen
 * saying why.
 *
 * ⚠ **A trailing `s` is a decade, not a year.** `1990s` runs to 1999; `1990`
 * is one year. Both spellings are authored, and collapsing them would make
 * every decade-spelled entry a single-year one — which is how `boom-bap` would
 * stop matching the 2000s pill.
 */
export function parseEra(era: string | null): Span | null {
  if (era === null) return null;
  const text = era.replace(/[–—]/g, '-').trim().toLowerCase().replace(QUALIFIER, '');
  const found = /^(\d{4})(s?)(?:-(present|\d{4}s?))?$/.exec(text);
  if (found === null) return null;

  const from = Number(found[1]);
  const decade = found[2] === 's';
  const end = found[3];

  if (end === undefined) return { from, to: decade ? from + 9 : from };
  if (end === 'present') return { from, to: STILL_CURRENT };

  const to = Number(end.slice(0, 4));
  return { from, to: end.endsWith('s') ? to + 9 : to };
}

/** Does a span touch the ten years starting at `decade`? */
export function overlaps(span: Span, decade: Decade): boolean {
  return span.from <= decade + 9 && span.to >= decade;
}

/**
 * Does `era` belong under any of the `decades` a producer has pressed?
 *
 * ⛔ **An empty selection is "no filter", never "nothing matches".** Pills that
 * hid the whole roster until one was pressed would make the list look broken on
 * load, which is the state it spends most of its life in.
 *
 * ⛔ **A style with no era, or one this cannot read, always shows.** Every one
 * of the 400 shipped models carries a parseable era, so in practice this is the
 * producer's *own* style (TASK-040U) — authored in an editor that does not ask
 * for one. Hiding somebody's own work behind a filter they were never offered a
 * way to satisfy is the worse of the two failures by a distance.
 */
export function matchesEras(era: string | null, decades: readonly Decade[]): boolean {
  if (decades.length === 0) return true;
  const span = parseEra(era);
  if (span === null) return true;
  return decades.some((decade) => overlaps(span, decade));
}
