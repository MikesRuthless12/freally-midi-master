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
};

/**
 * The page's own zoom. Set on the root element rather than in a stylesheet
 * because it is a property of the window this instance happens to be in, not
 * of the design — two instances of the plugin can be at different sizes.
 */
function applyZoom(zoom: number): void {
  document.documentElement.style.zoom = String(zoom);
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
    void invoke<SizeReply>('editor_size')
      .then((reply) => {
        setSizing(reply);
        applyZoom(reply.zoom);
      })
      .catch(() => {
        // A shell with no such command leaves the page at 1:1, which is what
        // it already was.
      });
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
            applyZoom(reply.zoom);
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
