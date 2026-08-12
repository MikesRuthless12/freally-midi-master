import { cleanup, render, screen, fireEvent, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { sectionsFor, useUi, WIDE_BREAKPOINT } from './state/ui';

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
  // ⚠ **Through `sectionsFor`, never by hand.** `sections` is derived from
  // `openGroups`, and a literal map here would seed a layout the app can no
  // longer reach — all eight panels at once, which is exactly the state the
  // rails were redesigned to stop existing.
  useUi.setState({
    activeTab: 'drums',
    rightRailOpen: true,
    theme: 'system',
    openGroups: { left: 0, right: 0 },
    sections: sectionsFor({ left: 0, right: 0 }),
    leaving: { left: null, right: null },
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

    // ⚠ **The genre combobox, not the search box.** The search box and the
    // five-hundred-row list were replaced by one type-to-filter combobox on
    // 2026-08-09 — so this asserts the control that now does that job rather
    // than being deleted, because "the left rail rendered something you can find
    // an artist with" is exactly what this test is for.
    expect(screen.getByRole('combobox', { name: 'Genres' })).toBeDefined();
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

  /**
   * ⛔⛔ **THE RAILS SWAP GROUPS; THEY DO NOT COLLAPSE PANELS** (2026-08-11).
   *
   * These three tests drove the accordion: click a panel's header, watch it
   * collapse, reopen it from the View menu. Mike replaced that model — *"only
   * leave 2 open at a time … file explorer's vertical tab replaces and takes the
   * place of both roster and genres"* — so there is no header toggle and no
   * collapsed state to persist. What is worth keeping from them is the
   * *property*: exactly one group per rail is mounted, the swap really unmounts
   * the old one, and the choice survives a relaunch.
   */
  it('swaps a whole group in from the rail tab, and unmounts the one it replaced', async () => {
    stubMatchMedia(1600);
    const { container } = render(<App />);

    // Kit and Stems are group 0 of the right rail, so both are up.
    expect(container.querySelector('#section-kit')).not.toBeNull();
    expect(container.querySelector('#section-stems')).not.toBeNull();
    expect(container.querySelector('#section-session')).toBeNull();

    // ⚠ The tab names what it will bring, not what is showing — Mike: *"then
    // switch the name on the vertical tab after they switch."*
    fireEvent.click(screen.getByRole('button', { name: /Session · Presets/i }));

    expect(container.querySelector('#section-session')).not.toBeNull();
    expect(container.querySelector('#section-presets')).not.toBeNull();
    // ⛔ The outgoing group stays mounted while it slides out, so this is the
    // state *after* the swap has finished — which is the one that matters, and
    // the one a leaked timer would break.
    await waitFor(() => expect(container.querySelector('#section-kit')).toBeNull());
    // The left rail is untouched: a swap is per rail.
    expect(container.querySelector('#section-roster')).not.toBeNull();
  });

  it('remembers which group each rail was showing across a remount', () => {
    stubMatchMedia(1600);
    const first = render(<App />);
    fireEvent.click(screen.getByRole('button', { name: /^Browser$/ }));
    expect(JSON.parse(window.localStorage.getItem('freally.railGroups')!).left).toBe(1);

    first.unmount();
    // Rehydrate the way a fresh launch would.
    useUi.setState({
      openGroups: { left: 1, right: 0 },
      sections: sectionsFor({ left: 1, right: 0 }),
      leaving: { left: null, right: null },
    });
    const again = render(<App />);

    expect(again.container.querySelector('#section-explorer')).not.toBeNull();
    expect(again.container.querySelector('#section-roster')).toBeNull();
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
      // TASK-045A. ⚠ Listed here as well as mounted, because a panel the View
      // menu cannot reach is a panel a producer who collapsed it cannot get
      // back — which is what this test exists to catch.
      'Pattern library',
    ]);
  });

  it('brings a group back from the View menu after the rail switched away', () => {
    stubMatchMedia(1600);
    render(<App />);

    // Switch the left rail to the browser, which is a group of one.
    fireEvent.click(screen.getByRole('button', { name: /^Browser$/ }));
    expect(useUi.getState().sections.roster).toBe(false);

    // ⚠ **The View menu is the second door and it still works**, which is the
    // point of keeping `sections` as a derived map: the menu names panels, not
    // groups, and clicking one brings whichever group holds it.
    fireEvent.click(screen.getByRole('button', { name: /View/i }));
    fireEvent.click(screen.getByRole('menuitemcheckbox', { name: /Roster/i }));

    expect(useUi.getState().sections.roster).toBe(true);
    expect(useUi.getState().sections.genres).toBe(true);
  });

  it('ignores K while typing in a field', () => {
    stubMatchMedia(1600);
    render(<App />);
    const before = useUi.getState().rightRailOpen;

    // Any text field in the rail proves the rule; the roster combobox is the one
    // that replaced the search box this test used to type into.
    const input = screen.getByRole('combobox', { name: 'Genres' });
    fireEvent.keyDown(input, { key: 'k' });

    expect(useUi.getState().rightRailOpen).toBe(before);
  });
});
