import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { KitPanel } from './KitPanel';
// ⚠ The mock's own idea of an unedited pad — one spelling, not two.
import { untouchedPad } from '../../lib/ipc-mock';
import { useKit } from '../../state/kit';

/**
 * The KIT panel tells the truth about what is loaded (TASK-136).
 *
 * ⛔⛔ **The defect this guards against was a readout that LIED, and Mike found
 * it in Ableton inside a minute of opening the plugin, 2026-08-04.**
 * `RightRail` rendered eight hardcoded `disabled` buttons and a static "No kit
 * yet" while a twelve-pad kit was loaded and *audibly playing*. The roadmap
 * called it "the most misleading thing in the UI": not an empty state but a
 * **wrong** state — the app telling the producer the opposite of the truth
 * about its own audio, so a reasonable person concludes the plugin is silent
 * because no kit is loaded and goes looking for one that was never missing.
 *
 * ⚠ **The wiring was fixed and nothing pinned it.** `KitPanel` has drawn every
 * word from `kit_state` for a while now, but there was no `KitPanel.test.tsx`
 * at all — so the one property that matters could have regressed in silence,
 * which is exactly how it got shipped the first time. These tests are that
 * guard.
 *
 * ⚠ Driven through the component with the store set directly, the same shape
 * `DragRows.test.tsx` uses: the store was never the thing that was wrong.
 */

/** The kit as the plugin reports it. `snap` is shipped-silent on purpose — the
 *  drum generator writes that lane and no shipped pad has ever played it. */
const KIT = [
  {
    lane: 'kick' as const,
    shipped: true,
    name: null,
    path: null,
    tweaks: untouchedPad(),
    reversed: false,
  },
  {
    lane: 'snare' as const,
    shipped: true,
    name: null,
    path: null,
    tweaks: untouchedPad(),
    reversed: false,
  },
  {
    lane: 'closedHat' as const,
    shipped: true,
    name: 'my-hat.wav',
    path: 'C:/s/my-hat.wav',
    tweaks: untouchedPad(),
    reversed: false,
  },
  {
    lane: 'snap' as const,
    shipped: false,
    name: null,
    path: null,
    tweaks: untouchedPad(),
    reversed: false,
  },
];

beforeEach(() => {
  // ⚠ `refresh` is stubbed rather than left real: the panel calls it on mount
  // and the real one reaches `invoke`, which would make every case here a test
  // of the IPC mock instead of the panel.
  useKit.setState({
    lanes: KIT,
    loaded: true,
    assigning: null,
    error: null,
    refresh: async () => {},
  });
});

afterEach(cleanup);

describe('the KIT panel says what is actually loaded', () => {
  it('never shows the silent-kit message while lanes are loaded', () => {
    // ⛔⛔ **THE REGRESSION GUARD, and it is the whole point of this file.**
    // This is the exact sentence the panel used to show — as a hardcoded
    // string, with no data behind it — while a full kit was playing.
    render(<KitPanel />);

    expect(
      screen.queryByText('The kit could not be loaded, so the plugin is silent.'),
    ).toBeNull();
    expect(screen.getByRole('list', { name: 'Kit lanes' })).toBeTruthy();
  });

  it('lists a row per lane the plugin reported, and no others', () => {
    // ⛔ Driven from what `kit_state` answered, never from a table written in
    // the component — a second list of lanes in the UI is the same defect with
    // an extra step.
    render(<KitPanel />);
    const lanes = screen.getByRole('list', { name: 'Kit lanes' });

    const names = within(lanes)
      .getAllByRole('listitem')
      .map((row) => row.getAttribute('data-lane'));
    expect(names).toEqual(['kick', 'snare', 'closedHat', 'snap']);
  });

  it('says what is playing each lane, and marks the one that can make no sound', () => {
    // The three facts a producer needs per row: which lane, what plays it, and
    // whether that is theirs. The old square pads had room for an index only.
    render(<KitPanel />);
    const lanes = screen.getByRole('list', { name: 'Kit lanes' });
    const row = (lane: string) => lanes.querySelector(`[data-lane="${lane}"]`) as HTMLElement;

    expect(row('kick').textContent).toContain('Built in');
    // The producer's own sample wins over the shipped one, and reads by name.
    expect(row('closedHat').textContent).toContain('my-hat.wav');
    expect(row('closedHat').getAttribute('data-assigned')).toBe('true');
    // ⚠ Shipped-silent: authored by the generator, played by nothing.
    expect(row('snap').textContent).toContain('No sound');
    expect(row('snap').getAttribute('data-silent')).toBe('true');
    expect(row('kick').getAttribute('data-silent')).toBe('false');
  });

  it('shows the silent-kit message only when the plugin reported no lanes', () => {
    // ⚠ **The honest empty state, and the only one.** It means the plugin could
    // not decode its own kit — not "no kit has been chosen", which was never a
    // thing this product had.
    useKit.setState({ lanes: [], loaded: true });
    render(<KitPanel />);

    expect(
      screen.getByText('The kit could not be loaded, so the plugin is silent.'),
    ).toBeTruthy();
    expect(screen.queryByRole('list', { name: 'Kit lanes' })).toBeNull();
  });

  it('says it is still reading rather than claiming silence before the first answer', () => {
    // ⛔ The distinction the old panel could not make: "not answered yet" is not
    // "answered, and the answer is nothing".
    useKit.setState({ lanes: [], loaded: false });
    render(<KitPanel />);

    expect(screen.getByText('Reading the kit…')).toBeTruthy();
    expect(
      screen.queryByText('The kit could not be loaded, so the plugin is silent.'),
    ).toBeNull();
  });

  /**
   * ⛔⛔ **HEARING ONE LANE'S SAMPLE ON ITS OWN** — Mike, 2026-08-11: *"the
   * melody/chords/basslines/counter melody should be able to play back their
   * samples with a play button as well, not just with the generated playback
   * pattern"* and *"you should be able to hear just the sample or one shot you
   * are using."*
   *
   * ▶ **The plugin could always do it; this list had no button.** `Audition::Lane`
   * triggers the pad **as sampled** — zero transposition, no generated part —
   * and the drum lanes have reached it from `PadGrid` and the grid's row headers
   * since TASK-043. The melodic lanes appear only here.
   */
  describe('auditioning one lane', () => {
    const MELODIC = [
      {
        lane: 'melody' as const,
        shipped: true,
        name: null,
        path: null,
        tweaks: untouchedPad(),
        reversed: false,
      },
      {
        lane: 'chords' as const,
        shipped: false,
        name: 'pad.wav',
        path: 'C:/s/pad.wav',
        tweaks: untouchedPad(),
        reversed: false,
      },
      {
        lane: 'bass' as const,
        shipped: true,
        name: null,
        path: null,
        tweaks: untouchedPad(),
        reversed: false,
      },
      {
        lane: 'counter' as const,
        shipped: true,
        name: null,
        path: null,
        tweaks: untouchedPad(),
        reversed: false,
      },
      // Nothing shipped and nothing assigned: there is no sample to hear.
      {
        lane: 'snap' as const,
        shipped: false,
        name: null,
        path: null,
        tweaks: untouchedPad(),
        reversed: false,
      },
    ];

    it.each(['melody', 'chords', 'bass', 'counter'])('offers Play on %s', (lane) => {
      // ⚠ All four named, not one standing for the rest: the gap was that a
      // whole class of lane had no route to a control, and a single example
      // would go on passing if three of them lost it again.
      useKit.setState({ lanes: MELODIC });
      render(<KitPanel />);
      const row = screen
        .getByRole('list', { name: 'Kit lanes' })
        .querySelector(`[data-lane="${lane}"]`) as HTMLElement;

      expect(within(row).getByRole('button', { name: /^Play / })).toBeTruthy();
    });

    it('withholds Play from a lane with no sample to play', () => {
      // ⚠ `canSound` is the shared predicate this row already keys `data-silent`
      // on. A Play button over a lane with no voice is a control that can only
      // do nothing — the readout-that-lies failure this panel exists to prevent.
      useKit.setState({ lanes: MELODIC });
      render(<KitPanel />);
      const row = screen
        .getByRole('list', { name: 'Kit lanes' })
        .querySelector('[data-lane="snap"]') as HTMLElement;

      expect(within(row).queryByRole('button', { name: /^Play / })).toBeNull();
    });
  });
});
