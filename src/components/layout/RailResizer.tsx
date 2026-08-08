import { useRef } from 'react';
import { useTranslation } from 'react-i18next';

import {
  RAIL_MAX_WIDTH,
  RAIL_MIN_WIDTH,
  clampRailWidth,
  useExplorer,
} from '../../state/explorer';

/**
 * The drag handle that widens the left rail (TASK-132).
 *
 * ⛔ **Mike, 2026-08-07:** *"ensure that the whole file explorer panel is able to
 * be resized so that you can see long file names and that the center panel
 * shrinks as you expand file explorer, but don't let it get absurdly wide."*
 * The centre shrinking is free: `.studio` is a grid whose first column is this
 * width and whose middle column is `minmax(0, 1fr)`, so every pixel the rail
 * takes is one the stage gives up. The ceiling is [`RAIL_MAX_WIDTH`] and the
 * floor exists so the handle can never be dragged out of reach of the gesture
 * that would bring it back.
 *
 * ⚠ **Pointer capture, not a `window` listener pair.** Capture keeps the moves
 * coming when the cursor outruns the 6px handle — which it always does — and it
 * ends on `pointerup` *and* on `pointercancel`, so a drag interrupted by the
 * host stealing focus does not leave the page stuck in a resize.
 *
 * ⚠ **Also a slider**, so this is not a mouse-only control: a rail that can only
 * be widened by dragging cannot be widened at all by anyone using the keyboard,
 * and the width is a real preference rather than a flourish.
 */
export function RailResizer() {
  const { t } = useTranslation();
  const railWidth = useExplorer((s) => s.railWidth);
  const setRailWidth = useExplorer((s) => s.setRailWidth);

  /**
   * The width this drag has reached, outside React.
   *
   * ⚠ Held in a ref rather than in state for the reason the move handler gives:
   * a render per pointer move is the thing being avoided. `null` between drags,
   * so a release that never moved commits nothing.
   */
  const dragging = useRef<number | null>(null);

  const commit = () => {
    const width = dragging.current;
    dragging.current = null;
    if (width === null) return;
    // ⚠ The inline property is left in place: it and the store now agree, and
    // clearing it first would flash the old width for one frame.
    setRailWidth(width);
  };

  return (
    <div
      className="rail__resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label={t('explorer.resize')}
      aria-valuenow={railWidth}
      aria-valuemin={RAIL_MIN_WIDTH}
      aria-valuemax={RAIL_MAX_WIDTH}
      tabIndex={0}
      onPointerDown={(event) => {
        // Only the primary button: a right-click here is the context menu, and
        // starting a resize on it leaves the menu open over a moving layout.
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
        // ⚠ Measured from the rail's own leading edge rather than by
        // accumulating deltas, so a frame dropped mid-drag cannot leave the
        // width permanently offset from the cursor.
        const rail = event.currentTarget.parentElement;
        if (!rail) return;
        const box = rail.getBoundingClientRect();
        // ⛔⛔ **Measured from the rail's LEADING edge, which is not always the
        // left one.** `src/i18n/index.ts` sets `dir="rtl"` for Arabic, and under
        // RTL the grid draws this rail on the *right* of the viewport with the
        // handle — `inset-inline-end` — on its visual left. `clientX - box.left`
        // then ran backwards: dragging outward to widen the rail *decreased*
        // `clientX`, so the width collapsed to the minimum, and because the
        // rail's own `left` moves as the width changes it re-measured against a
        // moving origin and juddered. An Arabic-locale producer could not widen
        // the browser at all — the one thing the handle exists for.
        const rtl = getComputedStyle(rail).direction === 'rtl';
        const width = rtl ? box.right - event.clientX : event.clientX - box.left;
        dragging.current = clampRailWidth(width);
        // ⛔⛔ **Written straight onto the document, NOT through the store.**
        // `railWidth` is read by `Studio`, which renders `LeftRail`,
        // `CenterStage` and `RightRail` — none of them memoised — so a `set()`
        // per pointer move reconciled the whole tree, including up to 320
        // timeline clips and up to 2,000 browser rows, at pointer rate. It also
        // ran a synchronous `localStorage.setItem` per move.
        //
        // ⚠ The value is only ever consumed as a custom property, and
        // `layout.css` reads it as `var(--rail-left-width, 280px)` — which
        // inherits from the root just as happily as from `.studio`. So the drag
        // paints at 60 fps with zero React work and the store learns the answer
        // once, on release. This is the same "a CSS variable precisely so it
        // does not have to re-render" rule `subscribeToPlayhead` and
        // `PreviewPlayer` already follow.
        document.documentElement.style.setProperty(
          '--rail-left-width',
          `${dragging.current}px`,
        );
      }}
      onPointerUp={(event) => {
        event.currentTarget.releasePointerCapture(event.pointerId);
        commit();
      }}
      onPointerCancel={(event) => {
        event.currentTarget.releasePointerCapture(event.pointerId);
        commit();
      }}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 48 : 16;
        if (event.key === 'ArrowLeft') setRailWidth(railWidth - step);
        else if (event.key === 'ArrowRight') setRailWidth(railWidth + step);
        else return;
        event.preventDefault();
      }}
    />
  );
}
