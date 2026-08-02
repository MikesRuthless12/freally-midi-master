import { useCallback, useEffect, useReducer, useRef } from 'react';

import { contrastRatio } from '../../lib/contrast';
import { useUi } from '../../state/ui';

/**
 * The colours the editor's three canvases paint with (TASK-041).
 *
 * ⛔ **Read from CSS custom properties, so both themes and the theme toggle work
 * without any canvas knowing either exists.** One copy: the roll, the velocity
 * lane and the ruler all draw the same six tokens, and when each had its own
 * `Palette` type and its own `readPalette` the three had already drifted — only
 * one of them listened for a *system* theme flip, and the fallback hexes were
 * three literals a token rename would have to find.
 *
 * ⛔ **Read once per theme, never per frame.** `getComputedStyle` forces a
 * synchronous style recalculation, and the draws run from a rAF scheduled right
 * after a React commit, when the style tree is dirty — so it was the single
 * most expensive thing in the roll's draw, six property reads deep, to fetch
 * values that only move when the theme does.
 */
export type Palette = {
  surface: string;
  surface2: string;
  border: string;
  primary: string;
  signal: string;
  text: string;
  text3: string;
  /**
   * The note outline, per fill, chosen for contrast (TASK-041F).
   *
   * ⛔ **Neither `text` nor `surface` clears 3:1 on both themes' note colours.**
   * Fixed at `text` the selection ring lands at 2.6:1 on the light theme's note
   * body, where "this note is selected" quietly stops being visible — and a
   * selection you cannot see is one you delete by accident. Decided here, once
   * per theme, rather than in the draw loop: it depends only on the palette, and
   * per-note it cost four luminance parses a note a frame.
   */
  outlineOnPrimary: string;
  outlineOnSignal: string;
};

/** Whichever of two colours reads better *on* a third. */
function readableOn(fill: string, a: string, b: string): string {
  return contrastRatio(fill, a) >= contrastRatio(fill, b) ? a : b;
}

export function readPalette(element: HTMLElement): Palette {
  const style = getComputedStyle(element);
  const get = (name: string, fallback: string) =>
    style.getPropertyValue(name).trim() || fallback;

  const surface = get('--color-surface', '#151519');
  const primary = get('--color-primary', '#8b5cf6');
  const signal = get('--color-signal', '#f59e0b');
  const text = get('--color-text', '#f4f4f5');

  return {
    surface,
    surface2: get('--color-surface-2', '#1e1e24'),
    border: get('--color-border', '#2c2c34'),
    primary,
    signal,
    text,
    text3: get('--color-text-3', '#71717a'),
    outlineOnPrimary: readableOn(primary, text, surface),
    outlineOnSignal: readableOn(signal, text, surface),
  };
}

/**
 * A palette that re-reads itself when the theme changes, and not otherwise.
 *
 * Returns a getter rather than a value: the draws are `useCallback`s that must
 * not be rebuilt when a colour changes, and a ref read inside one costs nothing.
 * `repaint` is the caller's own "draw again" — the palette cannot schedule a
 * paint it knows nothing about.
 *
 * ⛔ Both the store's theme *and* the system's are watched. `data-theme` covers
 * the in-app toggle; `prefers-color-scheme` covers the OS flipping under a
 * window left on "system", which two of the three canvases used to miss.
 */
export function useCanvasPalette(
  element: React.RefObject<HTMLElement | null>,
): () => Palette | null {
  const cached = useRef<Palette | null>(null);
  const theme = useUi((s) => s.theme);
  /**
   * Bumped whenever the palette is invalidated.
   *
   * ⛔ **A revision the reader's own identity depends on**, rather than a
   * repaint callback passed in. The callers' draws are `useCallback`s that
   * already list `paletteOf`, so making *it* change when the palette does is
   * what schedules the repaint — with nothing extra in a dependency array for a
   * linter to correctly call redundant, and no callback here that would tear
   * down and re-register the media listener on every frame of a drag.
   */
  const [revision, bump] = useReducer((n: number) => n + 1, 0);

  useEffect(() => {
    cached.current = null;
    bump();
  }, [theme]);

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => {
      cached.current = null;
      bump();
    };
    media.addEventListener('change', onChange);
    return () => media.removeEventListener('change', onChange);
  }, []);

  return useCallback(() => {
    const host = element.current;
    if (host === null) return null;
    return (cached.current ??= readPalette(host));
    // `revision` is the dependency: it is what makes this a new function when
    // the theme changes, and therefore what makes the caller's draw re-run.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [element, revision]);
}
