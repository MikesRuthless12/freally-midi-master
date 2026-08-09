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
import { isWide, useUi } from '../../state/ui';

/**
 * What the plugin says about the window it just sized.
 *
 * **No list of size names lives here.** `next` is what the button cycles to and
 * comes from `SCALES` in `plugin/src/editor.rs`, which is the only place the
 * names exist — a copy in TypeScript would let a rename there leave this button
 * asking for a size the plugin rejects.
 */
type SizeReply = {
  size: string;
  next: string;
  nextShrinks: boolean;
  width: number;
  height: number;
  zoom: number;
  /** What the page must lay out in — `LAYOUT` in `plugin/src/editor.rs`. */
  layoutWidth: number;
};

/**
 * The page's own zoom. Set on the root element rather than in a stylesheet
 * because it is a property of the window this instance happens to be in, not
 * of the design — two instances of the plugin can be at different sizes.
 */
function applyZoom(zoom: number): void {
  document.documentElement.style.zoom = String(zoom);

  // ⛔ Changing the zoom changes the width the app lays out in, but fires no
  // `resize` event — so the rail breakpoint has to be told, or it keeps the
  // answer it computed from the pre-zoom viewport. Without this the rail is
  // collapsed on open at every scale below 1.0, which is precisely what the
  // scaling exists to avoid.
  useUi.getState().setWide(isWide());
}

/**
 * The zoom the window we **actually got** needs, rather than the one the plugin
 * hoped for.
 *
 * ⛔⛔ **Mike, 2026-08-06:** *"when you go to a larger vst view, it has a bunch
 * of black, blank space outside of the VST3 itself's own main app window, that
 * should not happen."* The window size and the page zoom are one number that
 * has to be applied in two places, and they travel by different routes: the
 * zoom rides back on this reply and is applied at once, while the resize is
 * queued for the editor's frame loop and applied whenever that next runs. If
 * the window ends up any size other than the one the reply assumed — the loop
 * has not ticked yet, the host clamped it, or the display is a second monitor
 * at a different DPI, which `system_scale`'s own doc records as an unfixed
 * limit — the two disagree. Window larger than the layout is dead space around
 * the UI; smaller is a cropped app.
 *
 * ⚠ **`innerWidth` is deliberately the measurement, and it is the one that
 * works.** It is the *window's* CSS width and does not move when the root's
 * zoom changes (`state/ui.ts::isWide` records the measurement: at `zoom: 0.85`
 * in a 1224px window it stays 1224 while `clientWidth` reads 1440). So this is
 * idempotent — applying the result cannot change the next answer — and there is
 * no feedback loop to damp.
 *
 * `null` when there is nothing trustworthy to measure, and the caller then falls
 * back to the plugin's own figure.
 */
function measuredZoom(layoutWidth: number): number | null {
  const css = window.innerWidth;
  if (!Number.isFinite(css) || css <= 0) return null;
  if (!Number.isFinite(layoutWidth) || layoutWidth <= 0) return null;
  // ⚠ Bounded. A window reported as one pixel wide during a host's own resize
  // would otherwise set a zoom that makes the UI invisible, and the next honest
  // measurement is a whole frame away.
  const zoom = css / layoutWidth;
  return zoom >= 0.2 && zoom <= 4 ? zoom : null;
}

export function WindowSize() {
  const { t } = useTranslation();
  // `null` until the plugin answers, which is also what keeps the button from
  // guessing: the size is whatever the project was saved at, not a default
  // assumed here.
  const [sizing, setSizing] = useState<SizeReply | null>(null);

  // The window already opens at the saved size — the plugin reads it out of the
  // project state before it builds the editor — but the *page* has no way to
  // know what zoom that implies. Without this the first paint lays out 1:1
  // inside a scaled window and is cropped until the button is pressed.
  useEffect(() => {
    if (!isPlugin()) return;
    let live = true;
    let layout = 0;

    void invoke<SizeReply>('editor_size')
      .then((reply) => {
        if (!live) return;
        layout = reply.layoutWidth;
        setSizing(reply);
        applyZoom(measuredZoom(layout) ?? reply.zoom);
      })
      .catch(() => {
        // A shell with no such command leaves the page at 1:1, which is what
        // it already was.
      });

    // ⛔ **The resize arrives after the reply does, so the zoom has to be
    // re-derived when it lands.** `set_editor_size` queues the resize for the
    // editor's frame loop and answers immediately; the window changes some
    // frames later, if it changes at all. Correcting only at the moment of the
    // press would leave the page zoomed for a window it did not yet have —
    // which is the dead space this whole path exists to remove.
    const onResize = () => {
      if (layout === 0) return;
      const zoom = measuredZoom(layout);
      if (zoom !== null) applyZoom(zoom);
    };
    window.addEventListener('resize', onResize);
    return () => {
      live = false;
      window.removeEventListener('resize', onResize);
    };
  }, []);

  // Nothing is drawn until the plugin has said what size it is. A button that
  // guessed would show the wrong icon on a project saved at a different scale.
  if (!isPlugin() || sizing === null) return null;

  return (
    <button
      type="button"
      className="btn-ghost"
      aria-label={t('transport.windowSize')}
      title={t('transport.windowSize')}
      onClick={() => {
        // The reply is the source of truth, not what was asked for: the host
        // may clamp the size to the screen, and `fit` then returns the scale
        // that actually fits. Taking the answer rather than the request keeps
        // the zoom matched to the window it really got.
        //
        // The plugin records the choice in the project's own state, so there
        // is nothing to save here.
        void invoke<SizeReply>('set_editor_size', { size: sizing.next })
          .then((reply) => {
            setSizing(reply);
            // ⚠ The plugin's figure is the fallback, not the answer. The window
            // has almost certainly not resized yet at this point — the `resize`
            // listener above corrects it when it does — but applying the
            // measured value now keeps the page filling whatever it has in the
            // meantime rather than flashing dead space for a few frames.
            applyZoom(measuredZoom(reply.layoutWidth) ?? reply.zoom);
          })
          .catch(() => {
            // No such command in this shell; the window stays as it is.
          });
      }}
    >
      {sizing.nextShrinks ? (
        <Minimize2 size={14} aria-hidden="true" />
      ) : (
        <Maximize2 size={14} aria-hidden="true" />
      )}
    </button>
  );
}
