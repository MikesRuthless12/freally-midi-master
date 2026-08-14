import { cleanup, render, screen } from '@testing-library/react';
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
