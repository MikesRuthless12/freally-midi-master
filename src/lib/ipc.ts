/**
 * The single seam between the UI and the Rust core.
 *
 * Every `invoke` in the app goes through here, for one reason: Playwright can
 * then drive the real UI against `vite dev` with no backend at all. That keeps
 * E2E on the Linux CI runner cheap and, more importantly, keeps the tests
 * honest — they exercise the actual components rather than a stand-in.
 *
 * There are now three shells behind this one function, and the whole point of
 * having written it this way is that nothing above it had to change when the
 * project became a plugin:
 *
 * - **The plugin** — `ipc-plugin` talks to `plugin/src/bridge.rs` over the
 *   webview's message channel. This is what ships.
 * - **Tauri** — the desktop app, while it still exists.
 * - **Neither** — `ipc-mock`, for `vite dev` and Playwright.
 */

import type { InvokeArgs } from '@tauri-apps/api/core';

import { isPlugin } from './ipc-plugin';

/** True when running inside a Tauri WebView rather than a plain browser. */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** True when there is a real backend of either kind behind this UI. */
export function hasBackend(): boolean {
  return isTauri() || isPlugin();
}

/**
 * Whether to serve IPC from the mock.
 *
 * `VITE_IPC_MOCK=1` forces it on, which is what the Playwright config sets.
 * Otherwise the mock is used exactly when there is no backend to talk to — a
 * plain `vite dev` in a browser.
 */
function shouldUseMock(): boolean {
  if (import.meta.env.VITE_IPC_MOCK === '1') return true;
  return !hasBackend();
}

export async function invoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  if (shouldUseMock()) {
    // Loaded lazily so the mock and its fixtures never reach a production
    // bundle that runs inside a real shell.
    const { mockInvoke } = await import('./ipc-mock');
    return mockInvoke<T>(command, args);
  }
  if (isPlugin()) {
    const { pluginInvoke } = await import('./ipc-plugin');
    return pluginInvoke<T>(command, args);
  }
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke<T>(command, args);
}
