/**
 * The last thing between a render throw and a white window (TASK-093).
 *
 * ⛔⛔ **There was no boundary anywhere in `src/`, and in a plugin that is worse
 * than in a browser.** A React error unmounts the whole tree; in a hosted DAW
 * the producer gets a blank rectangle inside their project with no address bar,
 * no reload button and no console — the window is simply dead until they remove
 * and re-insert the plugin, losing the session. `plugin/src/bin/standalone.rs`
 * has a panic hook for the Rust half and its own comment says nih-plug replaces
 * it; the page half had nothing at all.
 *
 * ⛔ **A class, because there is no hook form of this.** `componentDidCatch` and
 * `getDerivedStateFromError` are the only API React offers, which is why this
 * one file is the only class component in the app.
 *
 * ⚠ **The strings are PROPS, not a `useTranslation()` call inside.** i18n is
 * initialised before first paint and is itself a plausible thing to have thrown
 * — a boundary that needs the failed subsystem to describe the failure shows a
 * blank window with extra steps. The wrapper resolves them where it is safe to,
 * and the defaults are English so the boundary works even then.
 */

import { Component, type ErrorInfo, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { FALLBACK_STRINGS, type ErrorBoundaryStrings } from './strings';
import './ErrorBoundary.css';

type Props = {
  children: ReactNode;
  strings?: ErrorBoundaryStrings;
  /** Told about every catch, so the Rust side can write it to the crash log. */
  onCaught?: (error: Error, componentStack: string) => void;
};

type State = { error: Error | null; componentStack: string };

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, componentStack: '' };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // ⛔ **Reported, never swallowed.** A boundary that renders a friendly
    // message and drops the stack turns a reproducible bug into a shrug — the
    // producer can describe the screen and nobody can find the throw.
    const stack = info.componentStack ?? '';
    this.setState({ componentStack: stack });
    this.props.onCaught?.(error, stack);
  }

  /**
   * ⚠ **Clearing the error is the whole recovery, and it is enough.** The state
   * of the app lives in the zustand stores outside this tree, so a remount
   * redraws from the same session rather than starting a new one. A full page
   * reload would be the heavier hammer and would lose the unsaved arrangement
   * this is trying to protect.
   */
  private retry = () => this.setState({ error: null, componentStack: '' });

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    const strings = this.props.strings ?? FALLBACK_STRINGS;
    return (
      <div className="crashpane" role="alert">
        <div className="crashpane__card">
          <h1 className="crashpane__title">{strings.title}</h1>
          <p className="crashpane__body">{strings.body}</p>
          <button type="button" className="crashpane__retry" onClick={this.retry}>
            {strings.retry}
          </button>
          {/* ⚠ Collapsed, not hidden. The producer does not have to read a stack
              trace, and the person they report it to must be able to. */}
          <details className="crashpane__details">
            <summary>{strings.details}</summary>
            <pre>
              {error.message}
              {this.state.componentStack}
            </pre>
          </details>
        </div>
      </div>
    );
  }
}

/**
 * The boundary with the producer's own language in it.
 *
 * ⚠ **`useTranslation` lives out here, above the boundary rather than inside
 * it.** A hook inside the class is impossible anyway, but the placement is also
 * the point: this component does not render the crashed subtree, so it cannot be
 * taken down by the same throw, and the strings are resolved before anything
 * that can fail has run.
 */
export function TranslatedErrorBoundary({
  children,
  onCaught,
}: {
  children: ReactNode;
  onCaught?: (error: Error, componentStack: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <ErrorBoundary
      onCaught={onCaught}
      strings={{
        title: t('crash.title', FALLBACK_STRINGS.title),
        body: t('crash.body', FALLBACK_STRINGS.body),
        retry: t('crash.retry', FALLBACK_STRINGS.retry),
        details: t('crash.details', FALLBACK_STRINGS.details),
      }}
    >
      {children}
    </ErrorBoundary>
  );
}
