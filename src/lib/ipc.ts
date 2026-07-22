/**
 * The single seam between the UI and the Rust core.
 *
 * Every `invoke` in the app goes through here, for one reason: Playwright can
 * then drive the real UI against `vite dev` with no Tauri binary at all. That
 * keeps E2E on the Linux CI runner cheap and, more importantly, keeps the
 * tests honest — they exercise the actual components rather than a stand-in.
 *
 * Outside Tauri, calls are served by `ipc-mock`. Inside Tauri, `@tauri-apps/api`
 * is used and the mock is never loaded.
 */

import type { InvokeArgs } from '@tauri-apps/api/core';

/** True when running inside a Tauri WebView rather than a plain browser. */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/**
 * Whether to serve IPC from the mock.
 *
 * `VITE_IPC_MOCK=1` forces it on, which is what the Playwright config sets.
 * Otherwise the mock is used exactly when there is no Tauri backend to talk
 * to — a plain `vite dev` in a browser.
 */
function shouldUseMock(): boolean {
  if (import.meta.env.VITE_IPC_MOCK === '1') return true;
  return !isTauri();
}

export async function invoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  if (shouldUseMock()) {
    // Loaded lazily so the mock and its fixtures never reach a production
    // bundle that runs inside Tauri.
    const { mockInvoke } = await import('./ipc-mock');
    return mockInvoke<T>(command, args);
  }
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke<T>(command, args);
}
