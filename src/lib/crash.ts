/**
 * Getting a render throw onto disk, where somebody can read it (TASK-093).
 *
 * ⛔⛔ **A boundary that shows a friendly message and drops the stack turns a
 * reproducible bug into a shrug.** The producer can describe the screen; nobody
 * can find the throw. So the pane recovers the window *and* this writes what
 * happened, in the same place the Rust panic hook writes its own log — one
 * folder a producer can be asked for, rather than two.
 */

import { invoke } from './ipc';

/**
 * Hand a caught render error to the plugin.
 *
 * ⛔ **Never throws, never rejects, and that is load-bearing rather than
 * defensive.** This runs from `componentDidCatch`, which is already the app's
 * last line — a rejection here would be an unhandled promise rejection raised
 * *by the error reporter*, on a path with nothing left to catch it. The console
 * is the fallback, which is what the browser build has anyway.
 *
 * ⚠ **The message and the component stack, not the `Error` object.** It crosses
 * a JSON bridge, and `Error` does not survive `structuredClone` with its stack
 * intact on every engine — sending the two strings is what makes the log say
 * the same thing the pane does.
 */
export function reportCrash(error: Error, componentStack: string): void {
  const detail = `${error.name}: ${error.message}\n${error.stack ?? ''}\n${componentStack}`;
  console.error('[crash]', detail);
  void invoke('report_crash', { detail }).catch(() => {
    // ⚠ Swallowed on purpose, and only here. The browser and Playwright builds
    // have no such command; a failed report must not replace the crash the
    // producer is looking at with a second one about the reporting.
  });
}
