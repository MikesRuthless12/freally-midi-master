import { isTauri } from '../../lib/ipc';

/**
 * Invisible grips around the window edge that start a native resize.
 *
 * A borderless window has no OS resize border — on Windows the frame that
 * normally provides the hit-test is gone entirely, so without these the window
 * simply cannot be resized. Each grip asks Tauri to begin a real resize drag,
 * so the OS does the work and snap behaviour is preserved.
 */

/** Matches Tauri's `ResizeDirection`; kept as strings so the API loads lazily. */
const GRIPS = [
  { dir: 'North', cls: 'n' },
  { dir: 'South', cls: 's' },
  { dir: 'East', cls: 'e' },
  { dir: 'West', cls: 'w' },
  { dir: 'NorthEast', cls: 'ne' },
  { dir: 'NorthWest', cls: 'nw' },
  { dir: 'SouthEast', cls: 'se' },
  { dir: 'SouthWest', cls: 'sw' },
] as const;

export function ResizeHandles() {
  if (!isTauri()) return null;

  const start = (direction: string) => async (e: React.PointerEvent) => {
    // Only the primary button, and never let the click reach the UI beneath.
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().startResizeDragging(direction as never);
    } catch {
      /* no window API — nothing to resize */
    }
  };

  return (
    <div className="resize" aria-hidden="true">
      {GRIPS.map(({ dir, cls }) => (
        <div
          key={cls}
          className={`resize__grip resize__grip--${cls}`}
          onPointerDown={start(dir)}
        />
      ))}
    </div>
  );
}
