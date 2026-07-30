import { useCallback, useEffect, useRef, useState } from 'react';

import { invoke } from '../../lib/ipc';
import { isPlugin } from '../../lib/ipc-plugin';
import './Eula.css';

/**
 * The licence gate ("any consent gate (EULA/first-run)").
 *
 * ⛔ **This is presentation, not enforcement.** The plugin refuses every command
 * at its RPC boundary until acceptance is recorded — see `licence_blocks` in
 * `plugin/src/editor.rs`. A page that was reloaded, bypassed or driven from
 * devtools still cannot generate. That separation is why this component is
 * allowed to fail open when the command does not exist: in a browser dev shell
 * there is no plugin to gate, and in the plugin the gate is not this file's job.
 *
 * **Deliberately not translated.** The app ships 18 locale catalogs and this
 * screen ignores all of them, because the agreement itself is English and a
 * machine-translated licence is a legal problem rather than an accessibility
 * win. The buttons match the text they act on.
 */

type EulaStatus = {
  version: string;
  text: string;
  accepted: boolean;
};

/** What the gate is doing, which is not the same question as "accepted". */
type Phase = 'checking' | 'reading' | 'declined' | 'done';

export function Eula({ children }: { children: React.ReactNode }) {
  // ⛔ Only the plugin is gated, and it starts resolved everywhere else —
  // synchronously, so the studio still mounts on the first render outside a
  // host. The gate exists to stop the *plugin* being used; a browser dev shell
  // has no bridge behind it to gate, and the desktop build is retired. Starting
  // in `checking` there would put a spinner in front of every dev session and
  // make the app's own mount asynchronous for no one's benefit.
  const [phase, setPhase] = useState<Phase>(() => (isPlugin() ? 'checking' : 'done'));
  const [status, setStatus] = useState<EulaStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  /**
   * Whether the agreement has actually been scrolled through.
   *
   * "You cannot use the app until you read it" is the requirement, and a button
   * that is live before the text has moved does not ask anyone to read anything.
   */
  const [read, setRead] = useState(false);
  const scroller = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!isPlugin()) return;
    let cancelled = false;

    invoke<EulaStatus>('eula_status')
      .then((next) => {
        if (cancelled) return;
        setStatus(next);
        setPhase(next.accepted ? 'done' : 'reading');
      })
      .catch(() => {
        // No such command: a browser dev shell or the desktop build. There is
        // no plugin behind this page to gate, and the plugin gates itself.
        if (!cancelled) setPhase('done');
      });

    return () => {
      cancelled = true;
    };
  }, []);

  /** A short agreement can be fully visible without ever scrolling. */
  const measure = useCallback(() => {
    const el = scroller.current;
    if (!el) return;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 24) setRead(true);
  }, []);

  useEffect(() => {
    if (phase === 'reading') measure();
  }, [phase, status, measure]);

  if (phase === 'done') return <>{children}</>;

  if (phase === 'checking') {
    return (
      <div className="eula" role="status">
        <p className="eula__checking">Checking the licence agreement…</p>
      </div>
    );
  }

  if (phase === 'declined') {
    return (
      <div className="eula" role="dialog" aria-modal="true" aria-labelledby="eula-declined">
        <div className="eula__panel eula__panel--narrow">
          <h1 id="eula-declined">Freally MIDI Master is not active</h1>
          <p>
            You declined the licence agreement, so nothing in the plugin will generate, play,
            export or save. Nothing you had already saved has been touched.
          </p>
          <p className="eula__muted">
            Read the agreement again and choose <strong>Agree</strong>, and the plugin works
            immediately.
          </p>
          <div className="eula__actions">
            <button
              type="button"
              className="eula__btn eula__btn--primary"
              onClick={() => {
                setRead(false);
                setPhase('reading');
              }}
            >
              Read the agreement
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="eula" role="dialog" aria-modal="true" aria-labelledby="eula-title">
      <div className="eula__panel">
        <h1 id="eula-title">Before you start</h1>
        <p className="eula__lede">
          <strong>
            You cannot use Freally MIDI Master until you have read this agreement and accepted
            its terms.
          </strong>{' '}
          Nothing will generate, play, export or save until you choose Agree.
        </p>

        <div className="eula__text" ref={scroller} onScroll={measure} tabIndex={0}>
          <pre>{status?.text ?? ''}</pre>
        </div>

        {error !== null && (
          <p className="eula__error" role="alert">
            {error}
          </p>
        )}

        <div className="eula__actions">
          <span className="eula__version">Version {status?.version ?? '—'}</span>
          <button
            type="button"
            className="eula__btn"
            onClick={() => {
              // Recorded rather than merely remembered in the page: a refusal
              // that a reload undoes is not a refusal.
              void invoke('eula_decline').catch(() => {});
              setPhase('declined');
            }}
          >
            Decline
          </button>
          <button
            type="button"
            className="eula__btn eula__btn--primary"
            disabled={!read}
            title={read ? undefined : 'Scroll to the end of the agreement first'}
            onClick={() => {
              setError(null);
              invoke('eula_accept')
                .then(() => setPhase('done'))
                .catch((reason: unknown) =>
                  setError(
                    `The plugin could not record your acceptance: ${
                      reason instanceof Error ? reason.message : String(reason)
                    }`,
                  ),
                );
            }}
          >
            Agree
          </button>
        </div>
        {!read && (
          <p className="eula__hint">Scroll to the end of the agreement to enable Agree.</p>
        )}
      </div>
    </div>
  );
}
