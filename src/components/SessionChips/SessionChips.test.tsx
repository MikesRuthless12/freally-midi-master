import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

const { SessionChips } = await import('./SessionChips');
const { useSession } = await import('../../state/session');

/**
 * The "Generate in" chip (TASK-158C).
 *
 * ⛔ **Here rather than in a Playwright spec**, because what has to hold is a
 * rule about the *roster* — which entries offer the chip — and a fixture is the
 * only way to control that. `e2e/generate-in.spec.ts` covers the gesture; this
 * covers who is offered it.
 */

const entry = (over: Partial<ReturnType<typeof artist>> = {}) => ({ ...artist(), ...over });

function artist() {
  return {
    id: 'mock-artist',
    name: 'Mock Artist',
    aliases: [] as string[],
    type: 'artist' as const,
    tier: 'flagship' as const,
    genres: ['trap'],
    relatedGenres: ['trap', 'uk-drill'],
    era: null,
    mine: false,
  };
}

const GENRES = [
  {
    ...artist(),
    id: 'trap',
    name: 'Trap',
    type: 'genre' as const,
    // ⛔ **A genre that carries `relatedGenres` too**, because 36 of the 56
    // shipped ones do. A fixture with none would make the rule below untestable
    // — it would pass whether or not the guard existed.
    relatedGenres: ['uk-drill'],
  },
  { ...artist(), id: 'uk-drill', name: 'UK Drill', type: 'genre' as const, relatedGenres: [] },
];

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(null);
  useSession.setState({
    selectedId: 'mock-artist',
    roster: [entry(), ...GENRES],
    base: null,
    defaults: null,
    patterns: {},
  });
});

afterEach(cleanup);

describe('the Generate in chip', () => {
  it('offers an artist the genres the roster lists them under', () => {
    render(<SessionChips />);
    expect(screen.getByRole('combobox', { name: 'Generate in' })).toBeTruthy();
  });

  it('is withheld from a genre, even one that carries relatedGenres', () => {
    // ⛔ 36 of 56 shipped genres carry them, so without the guard the chip
    // appears over Trap offering to generate "Trap, in Drill" — a control whose
    // meaning nobody asked for. The feature is an *artist* generating in a
    // genre they work in.
    useSession.setState({ selectedId: 'trap' });
    render(<SessionChips />);
    expect(screen.queryByRole('combobox', { name: 'Generate in' })).toBeNull();
  });

  it('is withheld from an artist who works in nothing else', () => {
    // ⚠ The same rule the mood chip follows: a combobox whose only entry is
    // "their own" is a control that cannot do anything.
    useSession.setState({
      selectedId: 'mock-artist',
      roster: [entry({ relatedGenres: [] }), ...GENRES],
    });
    render(<SessionChips />);
    expect(screen.queryByRole('combobox', { name: 'Generate in' })).toBeNull();
  });

  it('drops an id the roster does not name rather than showing the key', () => {
    // ⚠ `boom-bap` is a key, not a label. The plugin already drops dangling
    // `relatedGenres` from the roster, so this only ever loses one the rail is
    // not offering either — but showing the raw id would be worse than losing
    // it.
    useSession.setState({
      selectedId: 'mock-artist',
      roster: [entry({ relatedGenres: ['trap', 'gone'] }), ...GENRES],
    });
    render(<SessionChips />);
    const chip = screen.getByRole('combobox', { name: 'Generate in' });
    expect(chip).toBeTruthy();
    expect(screen.queryByText('gone')).toBeNull();
  });
});

/**
 * The Simple/Complex switch and the As Written switch (TASK-125).
 *
 * ⛔ **Three engine states behind two switches** (2026-08-16). `authored` is
 * still what the app opens in — it generates exactly what the app did before the
 * switch existed, so a saved seed still rebuilds its own beat — but it is no
 * longer a middle button a producer has to notice. It is a switch that visibly
 * disables the other one.
 *
 * ⚠ What the *engine* does with the setting is `engine/tests/complexity.rs`,
 * which measures it over the shipped roster. What only this can show is that the
 * two switches say which state they are in, disable each other in the one
 * direction they should, and write to the session.
 */
describe('the busy switch', () => {
  const side = () => screen.getByRole('switch', { name: 'Simple/Complex' });
  const held = () => screen.getByRole('switch', { name: 'As Written' });
  // ⚠ **`fireEvent`, not `element.click()`, and the difference is the point of
  // these tests.** A bare click leaves React un-rendered, so the *next* line
  // reaches the previous render's button — still disabled, still holding the
  // old handler — and a two-click sequence silently tests one click twice.
  const flip = (control: HTMLElement) => fireEvent.click(control);

  beforeEach(() => {
    useSession.setState({
      selectedId: 'mock-artist',
      roster: [entry(), ...GENRES],
      complexity: 'authored',
      lean: 'simple',
    });
  });

  it('starts on the model as written, with the other switch held off', () => {
    render(<SessionChips />);
    expect(held().getAttribute('aria-checked')).toBe('true');
    // ⛔ Disabled rather than absent: the control a producer cannot move must
    // still say which side it will hand back.
    expect((side() as HTMLButtonElement).disabled).toBe(true);
    expect(side().getAttribute('aria-checked')).toBe('false');
  });

  it('writes the producer’s choice to the session', () => {
    render(<SessionChips />);
    flip(held());
    expect(useSession.getState().complexity).toBe('simple');

    flip(side());
    expect(useSession.getState().complexity).toBe('complex');

    cleanup();
    render(<SessionChips />);
    // ...and the control says so on the way back, rather than only on the way in.
    expect(side().getAttribute('aria-checked')).toBe('true');
    expect(held().getAttribute('aria-checked')).toBe('false');
  });

  it('hands back the side it was on when As Written is turned off again', () => {
    // ⛔ The reason `lean` exists. Without it, `authored` would forget that the
    // producer was on Complex and turning the switch off would silently answer
    // Simple — a control that changes what an artist sounds like by being
    // switched on and off again.
    useSession.setState({ complexity: 'complex', lean: 'complex' });
    render(<SessionChips />);

    flip(held());
    expect(useSession.getState().complexity).toBe('authored');
    // ⚠ Still reading Complex while it is disabled, so what comes back is what
    // the knob is showing.
    expect(side().getAttribute('aria-checked')).toBe('true');
    expect((side() as HTMLButtonElement).disabled).toBe(true);

    flip(held());
    expect(useSession.getState().complexity).toBe('complex');
  });

  it('follows an undo that restores a side without going through the setter', () => {
    // ⚠ `applySnapshot` writes `complexity` straight into the store. Reading the
    // remembered lean unconditionally would leave the knob on Simple over a
    // session about to generate Complex — the readout-that-lies failure.
    useSession.setState({ complexity: 'complex', lean: 'simple' });
    render(<SessionChips />);
    expect(side().getAttribute('aria-checked')).toBe('true');
    expect((side() as HTMLButtonElement).disabled).toBe(false);
  });
});
