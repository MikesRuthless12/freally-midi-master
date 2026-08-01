/**
 * The single seam between the UI and the Rust core.
 *
 * Every `invoke` in the app goes through here, for one reason: Playwright can
 * then drive the real UI against `vite dev` with no backend at all. That keeps
 * E2E on the Linux CI runner cheap and, more importantly, keeps the tests
 * honest — they exercise the actual components rather than a stand-in.
 *
 * There are two shells behind this one function, and the whole point of having
 * written it this way is that nothing above it had to change when the project
 * became a plugin — or when the desktop shell was removed again:
 *
 * - **The plugin** — `ipc-plugin` talks to `plugin/src/editor.rs` over the
 *   webview's custom protocol. This is what ships.
 * - **Neither** — `ipc-mock`, for `vite dev` and Playwright.
 */

import { isPlugin } from './ipc-plugin';

/**
 * A command's arguments.
 *
 * Was `@tauri-apps/api/core`'s `InvokeArgs` while the desktop shell existed.
 * Declared here now that the dependency is gone: every call site passes a plain
 * object, and the bridge reads its arguments by name.
 */
export type InvokeArgs = Record<string, unknown>;

/**
 * Whether to serve IPC from the mock.
 *
 * `VITE_IPC_MOCK=1` forces it on, which is what the Playwright config sets.
 * Otherwise the mock is used exactly when there is no backend to talk to — a
 * plain `vite dev` in a browser.
 */
function shouldUseMock(): boolean {
  if (import.meta.env.VITE_IPC_MOCK === '1') return true;
  return !isPlugin();
}

export async function invoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  if (shouldUseMock()) {
    // Loaded lazily so the mock and its fixtures never reach a production
    // bundle that runs inside a real shell.
    const { mockInvoke } = await import('./ipc-mock');
    return mockInvoke<T>(command, args);
  }
  const { pluginInvoke } = await import('./ipc-plugin');
  return pluginInvoke<T>(command, args);
}
