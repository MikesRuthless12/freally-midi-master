/**
 * Draw the plugin window larger or smaller, without showing less of it.
 *
 * **Plugin only.** A desktop window is resized by dragging its edge and a
 * browser tab is not ours to resize, so this control has nothing to do in
 * either — `isPlugin()` is the same check `ipc.ts` uses to pick a shell.
 *
 * The important part is that this is a *scale*, not a smaller layout. The
 * plugin sizes the window to `LAYOUT * systemScale * factor` and sends back the
 * matching `zoom`; applying it here makes the page lay out at the full 1440x900
 * inside that smaller window. Shrink the window without the zoom and the right
 * rail collapses, which is the thing this is meant to avoid.
 *
 * `plugin/src/editor.rs` owns the factors; nothing here duplicates them, so the
 * two cannot drift apart.
 */

import { Maximize2, Minimize2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import { isPlugin } from '../../lib/ipc-plugin';

/** Smallest first, matching `SCALES` in the plugin. */
const SIZES = ['small', 'medium', 'large'] as const;
type Size = (typeof SIZES)[number];

/** What the editor opens at. `DEFAULT_SCALE` in the plugin says the same. */
const DEFAULT: Size = 'medium';

type SizeReply = { width: number; height: number; zoom: number };

/**
 * The page's own zoom. Set on the root element rather than in a stylesheet
 * because it is a property of the window this instance happens to be in, not
 * of the design — two instances of the plugin can be at different sizes.
 */
function applyZoom(zoom: number): void {
  document.documentElement.style.zoom = String(zoom);
}

async function askFor(size: Size): Promise<void> {
  try {
    const { zoom } = await invoke<SizeReply>('set_editor_size', { size });
    applyZoom(zoom);
  } catch {
    // A shell with no such command is not an error worth a toast — the window
    // simply stays as it is.
  }
}

export function WindowSize() {
  const { t } = useTranslation();
  const [size, setSize] = useState<Size>(DEFAULT);

  // The window already opens at the default size, but the *page* does not know
  // its zoom until someone says so — without this the first paint lays out at
  // 1:1 inside a window sized for 0.85 and everything is cropped.
  useEffect(() => {
    if (isPlugin()) void askFor(DEFAULT);
  }, []);

  if (!isPlugin()) return null;

  const next = SIZES[(SIZES.indexOf(size) + 1) % SIZES.length];

  return (
    <button
      type="button"
      className="btn-ghost"
      aria-label={t('transport.windowSize')}
      title={t('transport.windowSize')}
      onClick={() => {
        // Optimistic, and deliberately so: the window belongs to the host,
        // which may clamp the size to the screen or refuse it outright.
        // Tracking what was *asked for* keeps the button cycling either way,
        // rather than sticking on a size the host quietly declined.
        setSize(next);
        void askFor(next);
      }}
    >
      {size === 'large' ? (
        <Minimize2 size={14} aria-hidden="true" />
      ) : (
        <Maximize2 size={14} aria-hidden="true" />
      )}
    </button>
  );
}
