/**
 * Switch the plugin window between its sizes.
 *
 * **Plugin only.** A desktop window is resized by dragging its edge and a
 * browser tab is not ours to resize, so this control has nothing to do in
 * either — `isPlugin()` is the same check `ipc.ts` uses to pick a shell.
 *
 * Two presets rather than a drag handle because the vendored webview adapter
 * does not forward window-resize events; `plugin/src/editor.rs` carries the
 * full reason next to the sizes themselves.
 */

import { Maximize2, Minimize2 } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import { isPlugin } from '../../lib/ipc-plugin';

type Size = 'medium' | 'large';

export function WindowSize() {
  const { t } = useTranslation();
  const [size, setSize] = useState<Size>('medium');

  if (!isPlugin()) return null;

  const next: Size = size === 'medium' ? 'large' : 'medium';

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
        void invoke('set_editor_size', { size: next }).catch(() => {
          // A shell with no such command is not an error worth a toast — the
          // window simply stays as it is.
        });
      }}
    >
      {size === 'medium' ? (
        <Maximize2 size={14} aria-hidden="true" />
      ) : (
        <Minimize2 size={14} aria-hidden="true" />
      )}
    </button>
  );
}
