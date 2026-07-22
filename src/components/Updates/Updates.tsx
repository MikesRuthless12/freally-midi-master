import { useEffect, useState } from 'react';
import { Download, X } from 'lucide-react';

import { isTauri } from '../../lib/ipc';
import './Updates.css';
import { useTranslation } from 'react-i18next';

/**
 * The Havoc-standard update prompt (Part 2 of the standard).
 *
 * Rules this implements, each of which is a bug someone already shipped:
 *
 * - One check per launch, plus a manual one. Never a nag.
 * - Offline, rate-limited, or already current ⇒ **silent**. No toast, no
 *   "you're up to date!", nothing.
 * - The version and the real changelog come from the manifest, never from the
 *   download URL — macOS updater artifacts are named `<App>.app.tar.gz` with no
 *   version in the filename at all, so URL parsing works on two platforms and
 *   silently returns nothing on the third.
 * - Nothing downloads or installs without an explicit yes.
 * - A pending crash report outranks this dialog; App decides, and simply does
 *   not mount this component until the crash slot is free.
 * - Anything else that wants the slot temporarily — the bug dialog opened by
 *   hand — passes `hidden` rather than unmounting this component. Unmounting
 *   cancels the in-flight check and starts a fresh one on remount, which is two
 *   checks in a launch and, if the network dropped in between, no prompt at all.
 */

type Available = {
  version: string;
  notes: string;
  /** Runs the download+install. Resolves only on platforms that return. */
  install: () => Promise<void>;
};

type Phase = 'idle' | 'available' | 'installing' | 'failed';

export function UpdatePrompt({
  onDismiss,
  hidden = false,
}: {
  onDismiss: () => void;
  /** Yield the dialog slot without unmounting — see the header. */
  hidden?: boolean;
}) {
  const { t } = useTranslation();
  const [update, setUpdate] = useState<Available | null>(null);
  const [phase, setPhase] = useState<Phase>('idle');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    // No updater outside Tauri — a browser has nothing to update.
    if (!isTauri()) return;

    (async () => {
      try {
        const { check } = await import('@tauri-apps/plugin-updater');
        const found = await check();
        if (cancelled || !found) return; // already current — stay silent

        setUpdate({
          version: found.version,
          // The manifest's notes, which the release job fills from CHANGELOG.md.
          notes: found.body?.trim() || t('update.noNotes'),
          install: () => found.downloadAndInstall(),
        });
        setPhase('available');
      } catch {
        // Offline, rate-limited, DNS down, no release yet — all silent.
        // An update check that complains is worse than one that does nothing.
      }
    })();

    return () => {
      cancelled = true;
    };
    // `t` is read for the no-release-notes fallback. It is stable for a given
    // language, so this still runs once per launch — but if the user switches
    // language the note has to be re-read in the new one.
  }, [t]);

  // The check above still ran, and its result is held until the slot is free.
  if (hidden || phase === 'idle' || !update) return null;

  const install = async () => {
    setPhase('installing');
    setError(null);
    try {
      await update.install();
      // Reached only on macOS and Linux, where the bundle is swapped in place.
      // On Windows the updater calls exit(0) right after launching the NSIS
      // installer, so this never returns — that is by design, not a bug.
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    } catch (e) {
      setPhase('failed');
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="update" role="dialog" aria-modal="true" aria-labelledby="update-title">
      <div className="update__panel">
        <div className="update__head">
          <h2 id="update-title">
            <Download size={16} aria-hidden="true" />
            {t('update.available', { version: update.version })}
          </h2>
          <button
            type="button"
            className="btn-ghost"
            aria-label={t('common.close')}
            onClick={onDismiss}
            disabled={phase === 'installing'}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>

        <label className="update__label" htmlFor="update-notes">
          {t('update.whatsNew')}
        </label>
        <textarea id="update-notes" className="update__notes" readOnly value={update.notes} />

        {error && (
          <p className="update__error" role="alert">
            {t('update.installFailed', { error })}
          </p>
        )}

        <div className="update__actions">
          <button
            type="button"
            className="btn-ghost"
            onClick={onDismiss}
            disabled={phase === 'installing'}
          >
            {t('update.later')}
          </button>
          <button
            type="button"
            className="btn-generate"
            onClick={install}
            disabled={phase === 'installing'}
          >
            {phase === 'installing' ? t('update.installing') : t('update.install')}
          </button>
        </div>

        <p className="update__foot">{t('update.footer')}</p>
      </div>
    </div>
  );
}
