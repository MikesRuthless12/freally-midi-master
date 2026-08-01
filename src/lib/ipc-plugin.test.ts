import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { isPlugin, pluginInvoke } from './ipc-plugin';

/**
 * The transport between the UI and the plugin.
 *
 * It posts to the plugin over the same custom protocol the page was served
 * from, rather than the webview's IPC channel — see the module's own comment
 * for why. What these tests hold onto is that a failure is always *reported*:
 * a bridge that goes quiet has to surface as an error the user can read, not
 * as a spinner that never stops.
 */

const fetchMock = vi.fn();

beforeEach(() => {
  vi.stubGlobal('fetch', fetchMock);
  fetchMock.mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete window.sendToPlugin;
});

/** Answer as the plugin's RPC endpoint would. */
function answers(payload: unknown) {
  fetchMock.mockResolvedValue({ json: async () => payload } as Response);
}

/** Stand in for the marker the plugin's webview adapter injects. */
function installBridge() {
  window.sendToPlugin = () => {};
}

describe('isPlugin', () => {
  it('is false in a plain browser', () => {
    expect(isPlugin()).toBe(false);
  });

  it('is true when the webview adapter has injected its API', () => {
    installBridge();
    expect(isPlugin()).toBe(true);
  });
});

describe('pluginInvoke', () => {
  it('posts the command and resolves with what comes back', async () => {
    answers({ id: 1, ok: { entries: [] } });

    await expect(pluginInvoke('roster_summary', { a: 1 })).resolves.toEqual({
      entries: [],
    });

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('/__rpc');
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body as string)).toMatchObject({
      command: 'roster_summary',
      args: { a: 1 },
    });
  });

  it('gives every call its own id', async () => {
    answers({ ok: null });
    await pluginInvoke('one');
    await pluginInvoke('two');

    const ids = fetchMock.mock.calls.map(
      ([, init]) => (JSON.parse(init.body as string) as { id: number }).id,
    );
    expect(new Set(ids).size).toBe(2);
  });

  it('rejects with the plugin’s own message', async () => {
    answers({ id: 1, error: 'trap has no Melody part authored' });
    await expect(pluginInvoke('generate_pattern')).rejects.toThrow(
      'trap has no Melody part authored',
    );
  });

  it('reports a bridge that never answers instead of hanging', async () => {
    // The failure this exists for: without it a dropped call leaves a promise
    // pending for the life of the session, and the UI shows a spinner that
    // looks exactly like a slow generation.
    fetchMock.mockRejectedValue(new Error('TimeoutError'));
    await expect(pluginInvoke('roster_summary')).rejects.toThrow(/did not answer/);
  });

  it('names the command that failed', async () => {
    // "Something went wrong" in a plugin window, inside someone else's DAW,
    // with no console open, is not a bug report anybody can act on.
    fetchMock.mockRejectedValue(new Error('Failed to fetch'));
    await expect(pluginInvoke('host_session')).rejects.toThrow(/host_session/);
  });

  it('passes an empty object when there are no arguments', async () => {
    // The bridge indexes into `args`, and `undefined` would arrive as a
    // missing field rather than an empty one.
    answers({ ok: null });
    await pluginInvoke('app_info');
    expect(JSON.parse(fetchMock.mock.calls[0][1].body as string).args).toEqual({});
  });
});
