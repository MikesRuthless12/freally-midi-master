import { beforeEach, describe, expect, it } from 'vitest';

import { DEFAULT_PADS, RAIL_GROUPS, groupOf, sectionsFor, useUi } from './ui';

/**
 * The pad layout, and the rails' group model (2026-08-11).
 *
 * ⚠ **The store had no test file at all**, which is how a selector that returned
 * a fresh array every call reached `KitPanel` twice — once in `PadGrid` in
 * August, and again the day the KIT sort was written. The pad order is now
 * something a producer can *drag*, and a reordering that silently drops a lane
 * would be a pad they cannot get back.
 */

beforeEach(() => {
  window.localStorage.clear();
  useUi.setState({
    pads: {},
    selectedPad: 0,
    openGroups: { left: 0, right: 0 },
    sections: sectionsFor({ left: 0, right: 0 }),
    leaving: { left: null, right: null },
  });
});

describe('reordering the pads', () => {
  it('lifts the pad out and puts it back, rather than swapping two', () => {
    // ⛔ **Mike, 2026-08-11:** *"you should be able to click and drag your drum
    // pads and replace the ordering."* Swapping would move a lane the producer
    // never touched into the slot they dragged *from*, which reads as the grid
    // shuffling itself. Everything between closes up by one, like every other
    // reorderable list.
    useUi.getState().movePad('trap', 0, 3);

    expect(useUi.getState().pads.trap).toEqual([
      DEFAULT_PADS[1],
      DEFAULT_PADS[2],
      DEFAULT_PADS[3],
      DEFAULT_PADS[0],
      DEFAULT_PADS[4],
      DEFAULT_PADS[5],
      DEFAULT_PADS[6],
      DEFAULT_PADS[7],
    ]);
  });

  it('keeps every lane, so a drag can never lose one', () => {
    useUi.getState().movePad('trap', 7, 0);
    const after = useUi.getState().pads.trap;

    expect(after).toHaveLength(DEFAULT_PADS.length);
    expect([...after].sort()).toEqual([...DEFAULT_PADS].sort());
  });

  it('persists, so the order survives a reload', () => {
    // ⛔ Mike: *"the ordering should persist from unload to reload of the
    // app/vst."* Same key and same moment as a lane change, so there is one
    // answer to when a pad layout is written.
    useUi.getState().movePad('trap', 0, 1);

    const stored = JSON.parse(window.localStorage.getItem('freally.pads') ?? '{}');
    expect(stored.trap).toEqual(useUi.getState().pads.trap);
  });

  it('carries the selection with the pad it is on', () => {
    // ⚠ Otherwise dragging the selected pad silently re-aims `Ctrl`+arrow at
    // whatever slid into its slot.
    useUi.setState({ selectedPad: 0 });
    useUi.getState().movePad('trap', 0, 4);
    expect(useUi.getState().selectedPad).toBe(4);
  });

  it('shifts a selection the move stepped over', () => {
    // Pad 5 moves to slot 1, so the pad that was in slot 1 is now in slot 2.
    useUi.setState({ selectedPad: 1 });
    useUi.getState().movePad('trap', 5, 1);
    expect(useUi.getState().selectedPad).toBe(2);
  });

  it('refuses a move that would point outside the grid', () => {
    // ⚠ The only callers are the grid's own handlers, but an index past the end
    // is a layout with a hole in it — and a hole is a pad that plays nothing and
    // cannot be clicked.
    useUi.getState().movePad('trap', 0, 99);
    expect(useUi.getState().pads.trap).toBeUndefined();
  });

  it('still reorders before an artist has been chosen', () => {
    // ⛔⛔ **THIS ASSERTED THE OPPOSITE AND THE OPPOSITE WAS THE BUG.** It read
    // *"does nothing without a style to remember it against"* and expected an
    // untouched store — but `PadGrid` draws eight editable pads whether or not
    // `selectedId` is set (its own comment records why that state is reachable:
    // the roster combobox shows an artist before one is selected), so refusing
    // here is a drag that visibly snaps back with nothing said. Mike, 2026-08-11:
    // *"the drum pads do not let me reorder them, it just shows a selection
    // around the drum pad."* It is stored under `NO_STYLE` instead.
    useUi.getState().movePad(null, 0, 1);
    expect(useUi.getState().pads['']).toEqual([
      'snare',
      'kick',
      'clap',
      'closedHat',
      'openHat',
      'perc',
      'rim',
      'crash',
    ]);
  });
});

describe('the pad selection', () => {
  it('is never empty and never out of range', () => {
    // ⛔ Mike: *"one has to always be selected no matter what, so that way you
    // aren't trying to force one into something that's not there."*
    useUi.getState().selectPad(-3);
    expect(useUi.getState().selectedPad).toBe(0);

    useUi.getState().selectPad(99);
    expect(useUi.getState().selectedPad).toBe(DEFAULT_PADS.length - 1);
  });
});

describe('the rail groups', () => {
  it('shows exactly one group per rail', () => {
    const showing = sectionsFor({ left: 0, right: 0 });
    // Group 0 of each: genres + roster, kit + stems.
    expect(showing.genres).toBe(true);
    expect(showing.roster).toBe(true);
    expect(showing.explorer).toBe(false);
    expect(showing.kit).toBe(true);
    expect(showing.stems).toBe(true);
    expect(showing.session).toBe(false);
  });

  it('swaps the whole group when any of its panels is asked for', () => {
    // ⛔ Mike: *"session, presets and patterns show when you click and switch
    // places with kits and stems."* Pressing PRESETS brings all three.
    useUi.getState().showSection('presets');
    const { sections } = useUi.getState();

    expect(sections.session).toBe(true);
    expect(sections.presets).toBe(true);
    expect(sections.patterns).toBe(true);
    expect(sections.kit).toBe(false);
    // ⚠ And the other rail is untouched — a swap is per rail.
    expect(sections.roster).toBe(true);
  });

  it('puts every panel in exactly one group', () => {
    // ⚠ A panel missing from `RAIL_GROUPS` has no tab and no way in; a panel in
    // two would be shown by one rail and hidden by the other.
    const all = [...RAIL_GROUPS.left, ...RAIL_GROUPS.right].flat();
    expect(new Set(all).size).toBe(all.length);
    for (const id of all) {
      expect(groupOf(id).at).toBeGreaterThanOrEqual(0);
    }
  });
});
