import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { Combo } from '../Combo/Combo';
import { scoreEntry, search } from '../../lib/fuzzy';
import type { RosterEntry } from '../../lib/ipc-types';

/**
 * The roster UI at the size it actually ships (TASK-158E).
 *
 * ⛔⛔ **This is not a hypothetical bound.** `data/` carries **534** artists and
 * producers and **56** genres today — 590 roster entries — and the roadmap's own
 * words are that *"nothing has been run at 500+ entries"*. That is the gap these
 * tests close: the roster box was built when the dataset was a few dozen models
 * and had never been asked what it costs at the size the product now ships.
 *
 * ⛔ **What the measurement found, and it was not the DOM.** The rows are flat —
 * one `<li>` holding one `<button>` — and the menu is `max-height: 260px` with
 * `overflow-y: auto`, so what the browser paints is bounded however long the list
 * is. The cost was in `lib/fuzzy`, which runs over **every** entry on **every**
 * keystroke: it re-normalized the query once per entry, re-normalized all nine of
 * each entry's searchable fields, and split each of those into words twice. 5.5 ms
 * a keystroke at 590 entries, in the same frame React rebuilds the menu.
 * `fieldsOf`'s cache and hoisting the query's normalization took that to 2.9 ms.
 *
 * ⚠ **The timing assertion below is a smoke alarm, not a stopwatch.** Wall-clock
 * in a test measures the CI machine's afternoon as much as the code, so the bound
 * is an order of magnitude above what this costs — it exists to catch a change
 * that makes the search quadratic, and it will not flake on a busy runner.
 *
 * ⚠ **`RosterList.tsx` is deliberately not exercised here.** Nothing imports it:
 * the browsable list was replaced by this combobox on 2026-08-12 (*"the row is
 * gone"* — `LeftRail`), and the file is pre-existing dead code. Testing it would
 * make a component nobody renders look load-bearing.
 */

/** `count` roster entries with the field spread a real model has. */
function pool(count: number): RosterEntry[] {
  return Array.from({ length: count }, (_, at) => ({
    id: `artist-${at}`,
    name: `Some Artist Name ${at}`,
    aliases: [`alias one ${at}`, `alias two ${at}`, `alias three ${at}`],
    type: 'artist' as const,
    tier: null,
    genres: ['trap', 'drill', 'rap', 'hip hop'],
    relatedGenres: [],
    era: null,
    mine: false,
  }));
}

afterEach(cleanup);

describe('the roster combobox at 500+', () => {
  const options = Array.from({ length: 534 }, (_, at) => ({
    id: `artist-${at}`,
    name: `Artist ${at}`,
  }));

  it('does not build the list until it is opened', () => {
    // ⛔⛔ **The cost that would matter most, because it would be paid per
    // keystroke.** The menu is portalled and rebuilt whenever the narrowed list
    // changes, so a control that materialised its rows while closed would pay for
    // 534 of them on every character typed into a box not even showing them.
    render(<Combo label="Roster" options={options} value={null} onChange={() => {}} />);
    expect(screen.queryAllByRole('option')).toHaveLength(0);
  });

  it('narrows to what was typed rather than drawing everything under it', () => {
    // ⛔ **Typing is what makes 534 findable at all**, and it is also what keeps
    // the list cheap. A filter that highlighted instead of narrowing would leave
    // the whole roster drawn on every keystroke.
    render(<Combo label="Roster" options={options} value={null} onChange={() => {}} />);
    const field = screen.getByRole('combobox', { name: 'Roster' });
    field.focus();
    fireEvent.change(field, { target: { value: 'Artist 42' } });

    const shown = screen.getAllByRole('option');
    expect(shown.length).toBeLessThan(20);
    expect(shown.length).toBeGreaterThan(0);
    for (const option of shown) expect(option.textContent).toContain('Artist 42');
  });

  it('keeps a row flat when the whole roster is showing', () => {
    // ⚠ **Browsing the unfiltered list is a supported gesture**, not an edge
    // case: Mike's second way in is *"either show the list by clicking the arrow
    // on the right side of it, or type and it will autofill"*.
    //
    // ⛔ Three elements per row — `li > button > span`. A fourth is 534 more
    // nodes for the same list, and this is the assertion that notices.
    render(<Combo label="Roster" options={options} value={null} onChange={() => {}} />);
    const field = screen.getByRole('combobox', { name: 'Roster' });
    field.focus();
    fireEvent.change(field, { target: { value: 'Artist' } });

    expect(screen.getAllByRole('option')).toHaveLength(534);
    const menu = screen.getByRole('listbox', { name: 'Roster' });
    expect(menu.querySelectorAll('*').length).toBeLessThanOrEqual(534 * 3);
  });
});

describe('roster search at 500+', () => {
  it('answers a keystroke over the whole roster well inside a frame', () => {
    // ⛔ **The path `LeftRail` takes on every character**: `search(query, pool,
    // pool.length)` over artists *and* genres together, with no limit — narrowing
    // what you can find is the defect the cross-filter rule exists to prevent, so
    // the whole roster is scored every time.
    //
    // ⚠ The bound is ~17× what this costs today. See the module note: an order of
    // magnitude of headroom is what makes it a quadratic-catcher rather than a
    // flaky benchmark.
    const entries = pool(590);
    search('warm', entries, entries.length);

    const started = performance.now();
    for (let i = 0; i < 10; i += 1) search('trvis scott', entries, entries.length);
    const each = (performance.now() - started) / 10;

    expect(each).toBeLessThan(50);
  });

  it('caches an entry’s fields without changing what it scores', () => {
    // ⛔ **The cache is keyed on the entry object, and that is load-bearing.**
    // Saving a user model replaces the roster wholesale, so two objects with one
    // id is the normal case — and the *new* one is the one whose fields have to
    // be read. A cache keyed on `id` would answer a renamed style with its old
    // name for the life of the page.
    const [first] = pool(1);
    if (first === undefined) throw new Error('pool(1) has one entry');
    const before = scoreEntry('some artist', first);
    const again = scoreEntry('some artist', first);
    expect(again).toBe(before);

    const renamed: RosterEntry = { ...first, name: 'Totally Different' };
    expect(scoreEntry('totally different', renamed)).toBeGreaterThan(
      scoreEntry('totally different', first),
    );
  });
});
