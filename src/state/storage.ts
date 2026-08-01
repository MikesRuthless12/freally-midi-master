/**
 * Reading and writing the WebView's localStorage, in one place.
 *
 * Three modules were each carrying their own copy of this — theme, language and
 * the collapsed-panel state — right down to the same comment about private-mode
 * WebViews throwing on access. That tolerance is exactly the kind of thing that
 * gets hardened in one copy and forgotten in the other two.
 *
 * Note what this store is NOT: durable. It lives in the WebView profile, which
 * "clear browsing data" or an app-data restore can wipe. Nothing about a
 * *project* lives here — the session goes through the host's own state calls
 * (`plugin/src/state.rs`) and is saved with the song. What is here is the
 * view's own preferences, which are cheap to set again.
 */

/** Read a key, falling back when it is absent, invalid, or unreadable. */
export function readStored<T>(key: string, isValid: (v: unknown) => v is T, fallback: T): T {
  try {
    const stored = window.localStorage.getItem(key);
    if (isValid(stored)) return stored;
  } catch {
    // Private-mode or storage-disabled webviews throw on access.
  }
  return fallback;
}

/** Persist a value. Best-effort: the in-memory choice still applies if it fails. */
export function writeStored(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // As above — a webview that refuses storage must not break the app.
  }
}
