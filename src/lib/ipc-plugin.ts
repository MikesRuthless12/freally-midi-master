/**
 * The transport when the UI is running inside the plugin.
 *
 * The desktop app talked to Tauri; the plugin talks over the webview's own
 * message channel to `plugin/src/bridge.rs`. Same command names, same
 * payloads — `src/lib/ipc.ts` was always the single seam, and this is one more
 * thing behind it.
 *
 * Calls are correlated by id rather than answered in order, because the bridge
 * drains its queue on the editor's event loop and nothing promises that a slow
 * command finishes before a fast one sent after it.
 */

/** What the plugin's webview exposes. Neither exists in a browser or in Tauri. */
declare global {
  interface Window {
    ipc?: { postMessage: (message: string) => void };
    onPluginMessageInternal?: (json: unknown) => void;
    __TAURI_INTERNALS__?: unknown;
  }
}

type Pending = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
};

const pending = new Map<number, Pending>();
let nextId = 1;

/**
 * How long a call may go unanswered before it rejects.
 *
 * Without this a bridge that silently drops a message leaves a promise pending
 * for the life of the session, and the UI shows a spinner that never resolves
 * — indistinguishable from a slow generation. Generous, because the first call
 * of a session parses the whole dataset.
 */
const TIMEOUT_MS = 15_000;

/** True when running inside the plugin's webview. */
export function isPlugin(): boolean {
  if (typeof window === 'undefined') return false;
  // Tauri is checked first: its webview also has an `ipc` object, and treating
  // a Tauri session as a plugin one would route every call into a bridge that
  // is not there.
  if (window.__TAURI_INTERNALS__ !== undefined) return false;
  return typeof window.ipc?.postMessage === 'function';
}

/**
 * Install the reply handler. Idempotent — every call goes through here.
 *
 * Guarded on the handler *existing* rather than on a "have I done this yet"
 * flag. The two can disagree: a flag says installed while the property has
 * been replaced or cleared, and the transport then sends messages nothing will
 * ever answer — every call hanging until its timeout, with no error to
 * attribute it to.
 */
function listen(): void {
  if (window.onPluginMessageInternal) return;

  window.onPluginMessageInternal = (message: unknown) => {
    // The plugin sends a value, but a webview bridge may hand it over as a
    // JSON string depending on the platform. Accept both rather than failing
    // on one of the two operating systems.
    const payload = (typeof message === 'string' ? JSON.parse(message) : message) as {
      type?: string;
      id?: number;
      ok?: unknown;
      error?: string;
    };

    if (payload?.type !== 'response' || typeof payload.id !== 'number') return;

    const entry = pending.get(payload.id);
    if (!entry) return;
    pending.delete(payload.id);

    if (payload.error !== undefined) entry.reject(new Error(payload.error));
    else entry.resolve(payload.ok);
  };
}

export function pluginInvoke<T>(command: string, args?: unknown): Promise<T> {
  listen();

  const id = nextId++;
  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(() => {
      pending.delete(id);
      reject(new Error(`the plugin did not answer \`${command}\` within 15 seconds`));
    }, TIMEOUT_MS);

    pending.set(id, {
      resolve: (value) => {
        window.clearTimeout(timer);
        resolve(value as T);
      },
      reject: (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    });

    window.ipc?.postMessage(JSON.stringify({ id, command, args: args ?? {} }));
  });
}
