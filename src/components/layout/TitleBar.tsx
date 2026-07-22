import { useEffect, useState } from 'react';
import { Copy, Info, Minus, Settings, Square, X } from 'lucide-react';

import { isTauri } from '../../lib/ipc';
import { useTranslation } from 'react-i18next';

/**
 * The app's own title bar, since the window is borderless (`decorations: false`).
 *
 * Dragging is handled by Tauri itself through `data-tauri-drag-region` rather
 * than by tracking pointer events in JS — the native path keeps the OS snap
 * gestures (drag-to-maximise, snap to half-screen) that a hand-rolled one
 * silently loses.
 *
 * Double-clicking the bar toggles maximise, matching every platform's
 * convention.
 */

/** Lazily loaded so the browser build never imports the Tauri window API. */
async function windowApi() {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow();
}

export function TitleBar({
  onOpenSettings,
  onOpenAbout,
}: {
  onOpenSettings: () => void;
  onOpenAbout: () => void;
}) {
  const { t } = useTranslation();
  const [maximized, setMaximized] = useState(false);
  const native = isTauri();

  useEffect(() => {
    if (!native) return;
    let cancelled = false;

    const sync = async () => {
      try {
        const w = await windowApi();
        const is = await w.isMaximized();
        if (!cancelled) setMaximized(is);
      } catch {
        /* no window API — leave the icon as-is */
      }
    };

    void sync();

    // The window can also be maximised by an OS snap gesture or a keyboard
    // shortcut, so the icon has to follow the window rather than our clicks.
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const w = await windowApi();
        unlisten = await w.onResized(() => void sync());
      } catch {
        /* nothing to listen to */
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [native]);

  const minimize = async () => {
    if (native) (await windowApi()).minimize();
  };

  const toggleMaximize = async () => {
    if (!native) return;
    const w = await windowApi();
    await w.toggleMaximize();
    setMaximized(await w.isMaximized());
  };

  const close = async () => {
    if (native) (await windowApi()).close();
  };

  return (
    <header className="titlebar">
      {/* A full-width drag layer *underneath* the controls, rather than the
          controls living inside it. Tauri only treats the element carrying the
          attribute as draggable, so nesting a button inside would be fragile:
          any future change that let the event target resolve to the parent
          would turn the close button into a window drag. Siblings cannot. */}
      <div className="titlebar__drag" data-tauri-drag-region onDoubleClick={toggleMaximize} />

      {/* Centred on the window, not on the space left over beside the
          controls — so it stays centred whatever the controls do. */}
      <span className="titlebar__name">Freally MIDI Master</span>

      <div className="titlebar__controls">
        {/* App actions sit left of the window controls, so the close button
            stays in the corner where every platform puts it. */}
        <button
          type="button"
          className="titlebar__button titlebar__button--app"
          data-testid="open-settings"
          aria-label={t('titlebar.settings')}
          title={t('titlebar.settings')}
          onClick={onOpenSettings}
        >
          <Settings size={14} aria-hidden="true" />
        </button>

        <button
          type="button"
          className="titlebar__button titlebar__button--app"
          aria-label={t('titlebar.about')}
          title={t('titlebar.about')}
          onClick={onOpenAbout}
        >
          <Info size={14} aria-hidden="true" />
        </button>

        <span className="titlebar__divider" aria-hidden="true" />

        <button
          type="button"
          className="titlebar__button"
          aria-label={t('titlebar.minimize')}
          title={t('titlebar.minimize')}
          onClick={minimize}
        >
          <Minus size={14} aria-hidden="true" />
        </button>

        <button
          type="button"
          className="titlebar__button"
          aria-label={maximized ? t('titlebar.restore') : t('titlebar.maximize')}
          title={maximized ? t('titlebar.restore') : t('titlebar.maximize')}
          onClick={toggleMaximize}
        >
          {maximized ? (
            <Copy size={12} aria-hidden="true" />
          ) : (
            <Square size={12} aria-hidden="true" />
          )}
        </button>

        <button
          type="button"
          className="titlebar__button titlebar__button--close"
          aria-label={t('titlebar.close')}
          title={t('titlebar.close')}
          onClick={close}
        >
          <X size={14} aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}
