import { cleanup, render, screen, fireEvent } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { useUi, WIDE_BREAKPOINT } from './state/ui';

// No Rust backend under jsdom, so `src/lib/ipc` routes through `ipc-mock`
// automatically — the same path Playwright uses. Nothing to stub here.

/** jsdom ships no matchMedia. Drive it off a width we control. */
function stubMatchMedia(width: number) {
  const listeners = new Set<(e: MediaQueryListEvent) => void>();
  vi.stubGlobal('innerWidth', width);
  vi.stubGlobal(
    'matchMedia',
    (query: string): MediaQueryList =>
      ({
        media: query,
        matches: width >= WIDE_BREAKPOINT,
        addEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) =>
          listeners.add(cb),
        removeEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) =>
          listeners.delete(cb),
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
        onchange: null,
      }) as unknown as MediaQueryList,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  useUi.setState({
    activeTab: 'drums',
    rightRailOpen: true,
    theme: 'system',
    sections: {
      genres: true,
      roster: true,
      explorer: true,
      kit: true,
      stems: true,
      session: true,
      presets: true,
    },
  });
});

afterEach(() => {
  // Not automatic: Testing Library only self-cleans when vitest runs with
  // `globals: true`. Without this each render leaks into the next test.
  cleanup();
  vi.unstubAllGlobals();
});

describe('Studio shell', () => {
  it('renders every region of the layout', () => {
    stubMatchMedia(1600);
    render(<App />);

    expect(screen.getByLabelText('Search an artist')).toBeDefined();
    expect(screen.getByRole('tablist', { name: 'Generator' })).toBeDefined();
    // The transport is the region most easily lost to a grid-area mistake.
    expect(screen.getByRole('button', { name: 'Play' })).toBeDefined();
    // Settings and About hang off the transport bar rather than a title bar:
    // the host owns the plugin's window, so there is no chrome of ours to put
    // them on, and for a while there was no route to either at all.
    expect(screen.getByRole('button', { name: 'Settings' })).toBeDefined();
    expect(screen.getByRole('button', { name: 'About' })).toBeDefined();
  });

  it('shows all six generator tabs', () => {
    stubMatchMedia(1600);
    render(<App />);
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual([
      'Drums',
      'Melody',
      'Counter',
      'Bass',
      'Chords',
      'Song',
    ]);
  });

  it('moves the selection when a tab is clicked', () => {
    stubMatchMedia(1600);
    render(<App />);

    expect(screen.getByRole('tab', { name: 'Drums' }).getAttribute('aria-selected')).toBe(
      'true',
    );

    fireEvent.click(screen.getByRole('tab', { name: 'Chords' }));

    expect(screen.getByRole('tab', { name: 'Chords' }).getAttribute('aria-selected')).toBe(
      'true',
    );
    expect(screen.getByRole('tab', { name: 'Drums' }).getAttribute('aria-selected')).toBe(
      'false',
    );
  });

  it('marks the shell open when the right rail is showing', () => {
    stubMatchMedia(1600);
    const { container } = render(<App />);
    expect(container.querySelector('.studio')?.getAttribute('data-right-rail')).toBe('open');
    expect(container.querySelector('.rail--right')).not.toBeNull();
  });

  it('collapses the right rail below the breakpoint', () => {
    stubMatchMedia(1300);
    useUi.setState({ rightRailOpen: false });
    const { container } = render(<App />);
    expect(container.querySelector('.studio')?.getAttribute('data-right-rail')).toBe('closed');
    expect(container.querySelector('.rail--right')).toBeNull();
    // The transport must survive the collapsed layout too.
    expect(screen.getByRole('button', { name: 'Play' })).toBeDefined();
  });

  it('toggles the right rail with K', () => {
    stubMatchMedia(1300);
    useUi.setState({ rightRailOpen: false });
    const { container } = render(<App />);

    fireEvent.keyDown(window, { key: 'k' });
    expect(container.querySelector('.rail--right')).not.toBeNull();

    fireEvent.keyDown(window, { key: 'k' });
    expect(container.querySelector('.rail--right')).toBeNull();
  });

  it('collapses an individual panel from its header', () => {
    stubMatchMedia(1600);
    const { container } = render(<App />);

    const kit = screen.getByRole('button', { name: /Kit/i });
    expect(kit.getAttribute('aria-expanded')).toBe('true');
    // ⚠ The panel's *container*, not a string inside it. This used to assert on
    // "No kit yet" — which was the hardcoded placeholder TASK-136 removed, and
    // asserting on it meant this test would have kept passing however wrong the
    // panel's contents got. What is under test here is that collapsing unmounts
    // the content, and the content is what `Section` gives an id to.
    expect(container.querySelector('#section-kit')).not.toBeNull();

    fireEvent.click(kit);

    expect(kit.getAttribute('aria-expanded')).toBe('false');
    // Collapsed content is unmounted, not merely hidden.
    expect(container.querySelector('#section-kit')).toBeNull();
    // Collapsing one panel must not disturb its neighbours.
    expect(screen.getByRole('button', { name: /Session/i }).getAttribute('aria-expanded')).toBe(
      'true',
    );
  });

  it('persists collapsed panels across a remount', () => {
    stubMatchMedia(1600);
    const first = render(<App />);
    fireEvent.click(screen.getByRole('button', { name: /Genres/i }));
    expect(JSON.parse(window.localStorage.getItem('freally.sections')!).genres).toBe(false);

    first.unmount();
    // Rehydrate the way a fresh launch would.
    useUi.setState({
      sections: {
        genres: false,
        roster: true,
        explorer: true,
        kit: true,
        stems: true,
        session: true,
        presets: true,
      },
    });
    render(<App />);

    expect(screen.getByRole('button', { name: /Genres/i }).getAttribute('aria-expanded')).toBe(
      'false',
    );
  });

  it('lists every panel in the View menu', () => {
    stubMatchMedia(1600);
    render(<App />);

    fireEvent.click(screen.getByRole('button', { name: /View/i }));
    const items = screen.getAllByRole('menuitemcheckbox');
    expect(items.map((i) => i.textContent?.replace(/K$/, '').trim())).toEqual([
      'Right rail',
      'Genres',
      'Roster',
      'Browser',
      'Kit',
      'Stems',
      'Session',
      'Presets',
    ]);
  });

  it('reopens a panel from the View menu after it was collapsed', () => {
    stubMatchMedia(1600);
    render(<App />);

    fireEvent.click(screen.getByRole('button', { name: /Roster/i }));
    expect(useUi.getState().sections.roster).toBe(false);

    fireEvent.click(screen.getByRole('button', { name: /View/i }));
    fireEvent.click(screen.getByRole('menuitemcheckbox', { name: /Roster/i }));

    expect(useUi.getState().sections.roster).toBe(true);
  });

  it('ignores K while typing in a field', () => {
    stubMatchMedia(1600);
    render(<App />);
    const before = useUi.getState().rightRailOpen;

    const input = screen.getByLabelText('Search an artist');
    fireEvent.keyDown(input, { key: 'k' });

    expect(useUi.getState().rightRailOpen).toBe(before);
  });
});
