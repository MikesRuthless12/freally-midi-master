import { useEffect, useState } from 'react';

import {
  bugReportClearCrash,
  bugReportContext,
  bugReportPreview,
  bugReportSubmit,
  type BugReportContext,
  type BugReportTarget as Target,
} from './ipc';

import './BugReport.css';
import { useTranslation } from 'react-i18next';

/**
 * Report a bug — opt-in and anonymous. Shows the user the EXACT report (app/OS
 * + a scrubbed crash from the last run, if any), then lets them submit it via a
 * pre-filled GitHub issue, a Gmail compose window, or their own mail client.
 * The subject is `[Freally MIDI Master] <what went wrong>` so a report is
 * instantly attributable. No server, no shipped credentials — no report is
 * transmitted until the user clicks Send in their own client.
 */
export function BugReportOverlay({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [ctx, setCtx] = useState<BugReportContext | null>(null);
  const [description, setDescription] = useState('');
  const [includeCrash, setIncludeCrash] = useState(true);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    bugReportContext()
      .then(setCtx)
      .catch((err) => setError(String(err)));
  };

  useEffect(load, []);

  // Built by the same Rust function that sends it, so "exactly what will be
  // sent" is structural rather than two implementations promising to agree.
  // The TypeScript copy this replaces never applied `scrub()`, so the preview
  // could show a home path or username the real payload redacts — the two
  // disagreeing on precisely the axis this feature exists to guarantee.
  const [preview, setPreview] = useState('');
  useEffect(() => {
    if (!ctx) return;
    let cancelled = false;
    bugReportPreview(description, includeCrash && !!ctx.pendingCrash)
      .then((text) => {
        if (!cancelled) setPreview(text);
      })
      .catch((err) => setError(String(err)));
    return () => {
      cancelled = true;
    };
  }, [ctx, description, includeCrash]);

  const submit = (target: Target) => {
    setError(null);
    // Reload afterwards: submitting a report that included the crash clears it
    // on the Rust side, and the panel must stop offering to include a crash it
    // no longer holds.
    bugReportSubmit(target, description, includeCrash && !!ctx?.pendingCrash)
      .then(load)
      .catch((err) => setError(String(err)));
  };

  const copy = () => {
    navigator.clipboard
      .writeText(preview)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => setError(t('bugReport.copyFailed')));
  };

  const dismissCrash = () => {
    bugReportClearCrash()
      .then(load)
      .catch((err) => setError(String(err)));
  };

  return (
    <div className="bugreport-backdrop" onClick={onClose}>
      <div
        className="bugreport-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t('bugReport.title')}
        onClick={(event) => event.stopPropagation()}
      >
        <header className="bugreport-header">
          <h2>{t('bugReport.title')}</h2>
          <button
            type="button"
            className="btn-ghost"
            onClick={onClose}
            aria-label={t('common.close')}
          >
            ×
          </button>
        </header>

        <p className="bugreport-intro">{t('bugReport.intro')}</p>

        {ctx?.pendingCrash && (
          <p className="bugreport-crash-notice">{t('bugReport.crashNotice')}</p>
        )}

        <label className="bugreport-field">
          {t('bugReport.describeLabel')}
          <textarea
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            rows={3}
            placeholder={t('bugReport.describePlaceholder')}
          />
        </label>

        {ctx?.pendingCrash && (
          <label className="bugreport-check">
            <input
              type="checkbox"
              checked={includeCrash}
              onChange={(event) => setIncludeCrash(event.target.checked)}
            />
            {t('bugReport.includeCrash')}
          </label>
        )}

        <span className="bugreport-label">{t('bugReport.previewLabel')}</span>
        <pre className="bugreport-preview">{preview}</pre>

        <div className="bugreport-actions">
          <button type="button" className="btn-generate" onClick={() => submit('github')}>
            {t('bugReport.openGithub')}
          </button>
          <button
            type="button"
            className="btn-generate"
            onClick={() => submit('gmail')}
            title={t('bugReport.composeGmailTitle')}
          >
            {t('bugReport.composeGmail')}
          </button>
          <button
            type="button"
            className="btn-generate"
            onClick={() => submit('email')}
            title={t('bugReport.sendEmailTitle')}
          >
            {t('bugReport.sendEmail')}
          </button>
          <button type="button" className="btn-ghost" onClick={copy}>
            {copied ? t('bugReport.copied') : t('bugReport.copy')}
          </button>
          {ctx?.pendingCrash && (
            <button type="button" className="btn-ghost" onClick={dismissCrash}>
              {t('bugReport.dismissCrash')}
            </button>
          )}
        </div>

        {error && (
          <p role="alert" className="bugreport-error">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
