import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

import { ErrorBoundary } from './ErrorBoundary';
import { FALLBACK_STRINGS } from './strings';

/**
 * The crash pane (TASK-093).
 *
 * ⛔⛔ **In a hosted DAW a render throw is a dead rectangle inside the
 * producer's project** — no address bar, no reload, no console — and the only
 * way out is removing and re-inserting the plugin, which loses the session.
 * There was no boundary anywhere in `src/` before this.
 */

// React logs the caught error itself; the test output should not look like a
// failure when the failure is the point.
let noise: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
  noise = vi.spyOn(console, 'error').mockImplementation(() => {});
});
afterEach(() => {
  // ⛔ Unmounted between cases. Without it the second render finds two crash
  // panes and `getByRole('alert')` throws on the ambiguity — the repo's other
  // component tests all call this for the same reason.
  cleanup();
  noise.mockRestore();
});

function Boom({ throws }: { throws: boolean }) {
  if (throws) throw new Error('the piano roll fell over');
  return <p>the studio</p>;
}

it('renders its children when nothing has thrown', () => {
  render(
    <ErrorBoundary>
      <Boom throws={false} />
    </ErrorBoundary>,
  );
  expect(screen.getByText('the studio')).toBeTruthy();
});

it('shows a recovery pane instead of an empty window when a child throws', () => {
  render(
    <ErrorBoundary>
      <Boom throws />
    </ErrorBoundary>,
  );

  expect(screen.getByRole('alert')).toBeTruthy();
  expect(screen.getByText(FALLBACK_STRINGS.title)).toBeTruthy();
  // ⛔ The stack is kept, collapsed. A pane that showed a friendly message and
  // dropped it turns a reproducible bug into a shrug.
  expect(screen.getByText(/the piano roll fell over/)).toBeTruthy();
});

it('hands the error and the component stack to the reporter', () => {
  // ⚠ The Rust half writes this to `crash-{seconds}-page.log`. Losing it here
  // would mean the pane says one thing and the log on disk says nothing.
  const onCaught = vi.fn();
  render(
    <ErrorBoundary onCaught={onCaught}>
      <Boom throws />
    </ErrorBoundary>,
  );

  expect(onCaught).toHaveBeenCalledTimes(1);
  const [error, stack] = onCaught.mock.calls[0];
  expect((error as Error).message).toBe('the piano roll fell over');
  expect(typeof stack).toBe('string');
});

it('comes back to the app when the producer presses retry', () => {
  // ⛔ **This is the half that makes it a recovery rather than a nicer error
  // screen.** The session lives in the zustand stores outside this tree, so a
  // remount redraws the producer's own work — which is why the button clears the
  // error instead of reloading the page and losing the unsaved arrangement.
  //
  // ⚠ **The condition is flipped from OUTSIDE the component, and it has to be.**
  // A fixture that healed itself on re-render would never reach the pane at all:
  // React retries a failed render synchronously once, so a component that throws
  // only on its first call recovers before the boundary ever sees it.
  const control = { throws: true };
  function Controlled() {
    if (control.throws) throw new Error('the piano roll fell over');
    return <p>the studio</p>;
  }

  render(
    <ErrorBoundary>
      <Controlled />
    </ErrorBoundary>,
  );
  expect(screen.getByRole('alert')).toBeTruthy();

  control.throws = false;
  fireEvent.click(screen.getByText(FALLBACK_STRINGS.retry));

  expect(screen.getByText('the studio')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
});

it('falls back to English rather than to a blank pane', () => {
  // ⛔ i18n is initialised before first paint and is itself a plausible thing to
  // have thrown. A boundary that needs the failed subsystem to describe the
  // failure is a blank window with extra steps.
  render(
    <ErrorBoundary>
      <Boom throws />
    </ErrorBoundary>,
  );
  expect(screen.getByText(FALLBACK_STRINGS.body)).toBeTruthy();
});
