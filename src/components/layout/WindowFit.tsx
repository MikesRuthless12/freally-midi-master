/**
 * Fit the page to the plugin window it is in. **Nothing is drawn.**
 *
 * **Plugin only.** A browser tab is not ours to measure against a host window —
 * `isPlugin()` is the same check `ipc.ts` uses to pick a shell.
 *
 * The important part is that the window size and the page zoom are *one number*.
 * The plugin opens the window at `LAYOUT * systemScale * factor` and reports the
 * matching `zoom`; applying it here makes the page lay out at the full 1440x900
 * inside that window. Miss it and either the right rail collapses or there is
 * dead space around the interface.
 *
 * `plugin/src/editor.rs` owns the factors; nothing here duplicates them, so the
 * two cannot drift apart.
 */

import { useEffect } from 'react';

import { invoke } from '../../lib/ipc';
import { isPlugin } from '../../lib/ipc-plugin';
import { isWide, rootZoom, useUi } from '../../state/ui';

/**
 * What the plugin says about the window it opened.
 *
 * ⚠ **`editor_size` is the only command left that answers this**, and only two
 * of its fields are read now that the size button is gone. The rest are the
 * reply's own shape and are left named rather than trimmed: `plugin/src/editor.rs`
 * still sends them, and a type that lies about what arrives is worse than one
 * that describes more than it uses.
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
  const root = document.documentElement;
  root.style.zoom = String(zoom);

  // ⛔⛔ **THE APP MUST BE EXACTLY THE SIZE OF THE WINDOW.** Mike, 2026-08-09,
  // after three failed attempts at this from the Win32 side: *"will you PLEASE
  // ensure that the gui height = window.height"* — and length likewise.
  //
  // ⛔ **CSS `zoom` scales the rendered box, not only its contents**, and that is
  // the entire bug. `tokens.css` gives `html, body, #root` `height: 100%`; the
  // 100% resolves against the viewport and the result is *then* multiplied by
  // the zoom. At `zoom: 0.78` the app therefore paints 78% of the window in both
  // axes and leaves the remaining 22% showing the window behind it — the dead
  // space down the right and along the bottom, in exactly that proportion.
  //
  // ⚠ **Three attempts went into the webview's bounds looking for this.** The
  // webview was very likely the right size the whole time; the page inside it
  // was not filling it. Sizing the frame cannot fix a page that is painting at
  // 78% of whatever frame it is given.
  //
  // Dividing by the zoom is what cancels it: the box is laid out at
  // `window / zoom` CSS pixels and rendered at `window / zoom * zoom` — the
  // window, exactly, at any zoom.
  //
  // ⚠ **`innerWidth`/`innerHeight` are the right measurements** for the reason
  // [`measuredZoom`] gives below: they are the *window's* CSS size and do not
  // move when the root's zoom changes, so this is idempotent and cannot feed
  // back on itself.
  if (zoom > 0) {
    root.style.width = `${window.innerWidth / zoom}px`;
    root.style.height = `${window.innerHeight / zoom}px`;
  }

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

  // ⛔⛔ **Whichever axis is tighter, so nothing can ever be cut off.** Mike,
  // 2026-08-09: *"we need to set a minimum height/width to the size to where
  // everything is visible, even the right side panel if it is visible."*
  //
  // Width alone was not enough, and the failure is not exotic: the root is sized
  // to `window / zoom`, so with a width-only zoom a **short, wide** window gives
  // the page less than `LAYOUT` height however large it is — and the bottom of
  // the app is simply gone. Taking the smaller of the two ratios means the page
  // always gets **at least** 1440x900 in both directions; the spare pixels on
  // the other axis become extra room, which the layout already handles, rather
  // than something scrolled out of sight.
  //
  // ⚠ `LAYOUT`'s height lives in `plugin/src/editor.rs` and is not sent over the
  // bridge — only `layoutWidth` is. It is derived here from the 1440x900 ratio
  // rather than hardcoded twice, so a change there cannot leave this stale.
  const layoutHeight = layoutWidth * (900 / 1440);
  const zoom = Math.min(css / layoutWidth, window.innerHeight / layoutHeight);

  // ⚠ Bounded. A window reported as one pixel wide during a host's own resize
  // would otherwise set a zoom that makes the UI invisible, and the next honest
  // measurement is a whole frame away.
  return zoom >= 0.2 && zoom <= 4 ? zoom : null;
}

/**
 * What the page lays out in, learned from the first reply. `0` until it lands.
 *
 * ⚠ **Module scope rather than component state**, because `refit` is a bare
 * `resize` listener as well as `settle`'s body — a component-state read inside a
 * listener registered once would close over the value it had at mount. One
 * editor means one page, so there is nothing for two instances to fight over.
 */
let layoutWidth = 0;

/** The plugin's own figure, used only before the window can be measured. */
let repliedZoom = 1;

/** Re-fit the page to the window it *actually* has, right now. */
function refit(): void {
  if (layoutWidth === 0) return;
  // ⛔ **Applied even when the measurement is refused.** `measuredZoom` returns
  // `null` for a window it does not trust — mid-resize a host can report one
  // pixel — and skipping entirely used to mean the *size* was never re-applied
  // either, so the app stayed the shape of the window it had before.
  // ⚠ **`rootZoom` rather than reading `style.zoom` again**: it carries the same
  // 0.2–4 bound `measuredZoom` applies, so a figure a host left behind mid-resize
  // cannot be re-applied as though it were trustworthy. This component is the one
  // that *writes* that property, so it is the one place the two must agree.
  //
  // ⚠ Before the first pass there is nothing on the root to hold on to, and
  // `rootZoom` answers 1 for that — so the plugin's own figure is the seed.
  const held = document.documentElement.style.zoom === '' ? repliedZoom : rootZoom();
  applyZoom(measuredZoom(layoutWidth) ?? held);
}

let frames: number[] = [];
let timers: number[] = [];

/** Drop any scheduled re-fits. */
function cancelSettle(): void {
  frames.forEach(cancelAnimationFrame);
  timers.forEach(clearTimeout);
  frames = [];
  timers = [];
}

/**
 * Re-fit now, and keep re-fitting until the window has stopped moving.
 *
 * ⛔⛔ **A RESIZE THE PAGE IS NEVER TOLD ABOUT IS THE FAILURE THIS EXISTS FOR.**
 * Mike, 2026-08-12, on FL Studio's VST3: *"the VST3 doesn't resize right on the
 * Resize button"* — with a screenshot of a large window and the interface still
 * painted at its old size in the top-left corner, the rest of the frame blank.
 * He then removed the button entirely, which took the *request* away; what is
 * left is the handshake at open, and it has the same shape.
 *
 * ▶ **The window and the page's own box settle by two different routes.** The
 * plugin opens the window and answers `editor_size` at once, while baseview is
 * still bringing the window up and `fill_frame` is still sizing the webview to
 * it on the Win32 side. The page re-fits from its `resize` listener — **if one
 * fires**. When it does not, or when it fires before the host has finished, the
 * root keeps `window / zoom` computed for a window it had for one frame, and
 * that stale box is exactly the screenshot.
 *
 * ⚠ **This is why mount does not trust a single measurement**: *"when the app
 * first starts the gui is bigger than the window"* (Mike, 2026-08-09).
 *
 * ⚠ **Idempotent, so extra passes cost nothing and cannot oscillate.**
 * `innerWidth` does not move when the root's zoom does (see [`measuredZoom`]),
 * so once the window has settled every pass computes the same answer.
 */
function settle(): void {
  cancelSettle();
  refit();
  frames = [
    requestAnimationFrame(refit),
    requestAnimationFrame(() => requestAnimationFrame(refit)),
  ];
  // ⚠ Out to a second, because a host that resizes on its own message loop can
  // take that long — and a page left stale is not something the producer can
  // fix from inside the window.
  timers = [60, 160, 300, 600, 1000].map((ms) => window.setTimeout(refit, ms));
}

/**
 * Keep the page the size of its window. **Draws nothing.**
 *
 * ⛔⛔ **THE SIZE BUTTON IS GONE, AND THE BAR-COUNT RESIZE WITH IT** — Mike,
 * 2026-08-12: *"i also think we need to get rid of the resizing of the
 * vst3/clap/standalone, i think the smaller size is big enough, even for the 8
 * bars."* He said it straight after screenshotting FL Studio's VST3 with the
 * window grown and the interface still painted at its old size in the corner.
 *
 * ▶ **Asking a host to resize a plugin editor is the part that does not work
 * the same way twice.** Ableton honours it, FL docks and does not, and every
 * fix for one has broken the other — `host_frame` in the vendored adapter
 * carries three of those in a row. Nothing now calls `set_editor_size`, so
 * there is no request to be honoured differently and no window that can
 * disagree with its contents.
 *
 * ⚠ **What is deliberately kept:** the window still *opens* at whatever the
 * project saved, the producer can still drag a standalone window's edge, and
 * this component still fits the page to whatever it is given. That fitting is
 * not the feature being removed — it is what stops the dead space.
 *
 * ⚠ **The Rust side is left standing.** `SCALES`, `set_editor_size` and
 * `next_scale` in `plugin/src/editor.rs` are now unreached from the UI. Deleting
 * them is a separate change with its own tests, and a project saved at `xl`
 * still has to open.
 */
export function WindowFit() {
  // The window already opens at the saved size — the plugin reads it out of the
  // project state before it builds the editor — but the *page* has no way to
  // know what zoom that implies. Without this the first paint lays out 1:1
  // inside a scaled window and is cropped.
  useEffect(() => {
    if (!isPlugin()) return;
    let live = true;

    void invoke<SizeReply>('editor_size')
      .then((reply) => {
        if (!live) return;
        layoutWidth = reply.layoutWidth;
        repliedZoom = reply.zoom;
        // ⛔⛔ **The window is not its final size when the page first measures
        // it.** Mike, 2026-08-09: *"when the app first starts the gui is bigger
        // than the window."* The reply arrives while baseview is still opening
        // the window, so the first measurement reads a viewport that is about to
        // change — and because nothing resizes it afterwards, no `resize` event
        // ever arrives to correct the guess. `settle` is that train of passes.
        //
        // ⚠ **A `ResizeObserver` on the root cannot serve**: `applyZoom` sets
        // the root's own width and height, so observing it would see its own
        // writes. These re-measure the *window*, which nothing here changes.
        settle();
      })
      .catch(() => {
        // A shell with no such command leaves the page at 1:1, which is what
        // it already was.
      });

    // ⛔ **The window can still change after the reply, and this is what hears
    // it.** Nothing in the app asks a host to resize any more, but the producer
    // can drag a standalone window's edge and a host can resize a docked editor
    // on its own — and either leaves the page laid out for the window it used to
    // have, which is the dead space this whole path exists to remove.
    //
    // ⚠ One pass rather than a whole `settle`, because this fires on every frame
    // of a drag and rescheduling the train each time would be work nobody needs.
    // `settle` is for the moment that has *no* event: the opening handshake.
    window.addEventListener('resize', refit);

    return () => {
      live = false;
      window.removeEventListener('resize', refit);
      cancelSettle();
    };
  }, []);

  return null;
}
