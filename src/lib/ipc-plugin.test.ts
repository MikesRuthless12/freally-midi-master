import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { isPlugin, pluginInvoke } from './ipc-plugin';

/**
 * The transport between the UI and the plugin.
 *
 * Everything here is about the two ways this layer can lie: answering the
 * wrong caller, and never answering at all. A webview bridge has no ordering
 * guarantee and no failure signal, so both are silent by default.
 */

type Sent = { id: number; command: string; args: unknown };

/** What the plugin's webview injects, and what the tests stand in for. */
function installBridge(): Sent[] {
  const sent: Sent[] = [];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).ipc = {
    postMessage: (message: string) => sent.push(JSON.parse(message) as Sent),
  };
  return sent;
}

/** Answer as the plugin would. */
function reply(payload: unknown, asString = false) {
  const handler = window.onPluginMessageInternal;
  expect(handler, 'the transport should have installed a reply handler').toBeDefined();
  handler!(asString ? JSON.stringify(payload) : payload);
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  delete (window as any).ipc;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  delete (window as any).__TAURI_INTERNALS__;
  delete window.onPluginMessageInternal;
});

describe('isPlugin', () => {
  it('is false in a plain browser', () => {
    expect(isPlugin()).toBe(false);
  });

  it('is true when the webview bridge is present', () => {
    installBridge();
    expect(isPlugin()).toBe(true);
  });

  it('is false inside Tauri even though it also has an ipc object', () => {
    // The bug this prevents: Tauri's webview exposes `ipc` too, so a naive
    // check routes every desktop call into a plugin bridge that is not there
    // — and every one of them hangs until the timeout.
    installBridge();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (window as any).__TAURI_INTERNALS__ = {};
    expect(isPlugin()).toBe(false);
  });
});

describe('pluginInvoke', () => {
  it('sends the command and resolves with what comes back', async () => {
    const sent = installBridge();
    const call = pluginInvoke<{ ok: true }>('roster_summary', { a: 1 });

    expect(sent).toHaveLength(1);
    expect(sent[0].command).toBe('roster_summary');
    expect(sent[0].args).toEqual({ a: 1 });

    reply({ type: 'response', id: sent[0].id, ok: { ok: true } });
    await expect(call).resolves.toEqual({ ok: true });
  });

  it('answers each caller with its own reply, whatever order they arrive in', async () => {
    // The bridge drains its queue on the editor's event loop and nothing
    // promises a slow command finishes before a fast one sent after it. Match
    // by id, or a generation resolves with the roster.
    const sent = installBridge();
    const first = pluginInvoke<string>('slow');
    const second = pluginInvoke<string>('fast');

    reply({ type: 'response', id: sent[1].id, ok: 'second' });
    reply({ type: 'response', id: sent[0].id, ok: 'first' });

    await expect(first).resolves.toBe('first');
    await expect(second).resolves.toBe('second');
  });

  it('rejects with the plugin’s own message', async () => {
    const sent = installBridge();
    const call = pluginInvoke('generate_pattern');

    reply({ type: 'response', id: sent[0].id, error: 'trap has no Melody part authored' });
    await expect(call).rejects.toThrow('trap has no Melody part authored');
  });

  it('accepts a reply delivered as a JSON string', async () => {
    // Which of the two a webview hands over is platform-dependent, and
    // supporting only one means the plugin works on exactly one OS.
    const sent = installBridge();
    const call = pluginInvoke<number>('host_session');

    reply({ type: 'response', id: sent[0].id, ok: 92 }, true);
    await expect(call).resolves.toBe(92);
  });

  it('rejects rather than hanging when nothing ever answers', async () => {
    // Without this a dropped message leaves a promise pending for the life of
    // the session, and the UI shows a spinner that never resolves — which
    // looks exactly like a slow generation.
    installBridge();
    const call = pluginInvoke('generate_pattern');
    const assertion = expect(call).rejects.toThrow(/did not answer/);

    await vi.advanceTimersByTimeAsync(15_001);
    await assertion;
  });

  it('ignores a reply for a call that already resolved', async () => {
    // A duplicate must not throw on the way through and take the whole
    // handler down with it, silently ending every future reply.
    const sent = installBridge();
    const call = pluginInvoke<string>('app_info');

    reply({ type: 'response', id: sent[0].id, ok: 'once' });
    await expect(call).resolves.toBe('once');

    expect(() => reply({ type: 'response', id: sent[0].id, ok: 'twice' })).not.toThrow();
  });

  it('ignores messages that are not responses', async () => {
    const sent = installBridge();
    const call = pluginInvoke<string>('app_info');

    expect(() => reply({ type: 'something-else', id: sent[0].id })).not.toThrow();
    reply({ type: 'response', id: sent[0].id, ok: 'still works' });
    await expect(call).resolves.toBe('still works');
  });
});
