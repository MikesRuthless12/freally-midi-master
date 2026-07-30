/**
 * The transport when the UI is running inside the plugin.
 *
 * The desktop app talked to Tauri; the plugin posts to `plugin/src/editor.rs`
 * over the same custom protocol the page itself was served from. Same command
 * names, same payloads — `src/lib/ipc.ts` was always the single seam, and this
 * is one more thing behind it.
 *
 * **Why `fetch` and not the webview's IPC channel.** wry's IPC is one-way: a
 * reply has to be pushed back with `evaluate_script`, which the adapter only
 * does from the editor's *frame loop* — and a plugin window parented into
 * Ableton Live never gets a frame tick. Every command was sent and none was
 * ever answered, with nothing in the console but the sends. A custom-protocol
 * request is handled synchronously and returns a body, so a call and its reply
 * are one exchange that depends on no tick at all.
 */

/** What the plugin's webview injects. Neither exists in a browser or in Tauri. */
declare global {
  interface Window {
    /** Injected by the plugin's webview adapter — the marker we detect on. */
    sendToPlugin?: (message: unknown) => void;
    __TAURI_INTERNALS__?: unknown;
  }
}

/**
 * Where commands are posted. Same origin as the page, so no scheme is needed
 * and the browser resolves it against whatever the host mapped the protocol to
 * — `http://freally.localhost/` on Windows, `freally://` elsewhere.
 */
const RPC_URL = '/__rpc';

/**
 * How long a call may go unanswered before it gives up.
 *
 * Generous, because the first call of a session parses the whole dataset.
 */
const TIMEOUT_MS = 15_000;

let counter = 1;

/** True when running inside the plugin's webview. */
export function isPlugin(): boolean {
  if (typeof window === 'undefined') return false;
  // Tauri is checked first: its webview also exposes an `ipc` object, and
  // treating a Tauri session as a plugin one would route every call into a
  // bridge that is not there.
  if (window.__TAURI_INTERNALS__ !== undefined) return false;
  return typeof window.sendToPlugin === 'function';
}

export async function pluginInvoke<T>(command: string, args?: unknown): Promise<T> {
  const id = counter++;

  // `AbortSignal.timeout` rather than a bare fetch: without it a request the
  // plugin never answers leaves a promise pending for the life of the session,
  // and the UI shows a spinner that never resolves — indistinguishable from a
  // slow generation.
  let response: Response;
  try {
    // Not a network call, and the ban that flags it is right to exist. This
    // request never leaves the process: `freally://` is a custom protocol
    // served by `plugin/src/editor.rs` in the same binary, with no socket, no
    // DNS and no host beyond the webview itself. It is the plugin's IPC, using
    // the one transport that works in a hosted window — see the module comment.
    //
    // eslint-disable-next-line no-restricted-globals
    response = await fetch(RPC_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id, command, args: args ?? {} }),
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
  } catch (error) {
    // Both the message and the `cause`. A bare "the plugin did not answer"
    // inside a DAW, with no console open, is not something anyone can act on.
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(`the plugin did not answer \`${command}\`: ${reason}`, {
      cause: error,
    });
  }

  const payload = (await response.json()) as { ok?: unknown; error?: string };
  if (payload.error !== undefined) throw new Error(payload.error);
  return payload.ok as T;
}
