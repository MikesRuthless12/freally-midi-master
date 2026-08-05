import { beforeEach, expect, it, vi } from 'vitest';

import type { Lane } from '../lib/ipc-types';

/**
 * The KIT panel's store (TASK-131B, TASK-136).
 *
 * ⛔ **What these exist for is the poll.** Everything else here is a
 * pass-through to the bridge; the assignment loop is the part with a shape of
 * its own, and it is the part that can strand the panel — a poll that gives up
 * leaves "Choosing…" on screen forever, and a poll that treats a closed dialog
 * as a failure puts an error under a producer who simply changed their mind.
 * Both of those are failures this repo has already shipped once, in the export.
 */

const invoke = vi.fn();
vi.mock('../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

const { useKit, ALL_LANES } = await import('./kit');

/** A `kit_state` reply with `assigned` carrying the producer's own samples. */
function kitState(assigned: Partial<Record<Lane, string>> = {}) {
  return {
    id: 'trap-default',
    lanes: ALL_LANES.map((lane) => ({
      lane,
      shipped: lane !== 'snap',
      name: assigned[lane] ?? null,
      path: assigned[lane] ? `C:/samples/${assigned[lane]}` : null,
    })),
  };
}

beforeEach(() => {
  invoke.mockReset();
  useKit.setState({ id: null, lanes: [], loaded: false, assigning: null, error: null });
  vi.useRealTimers();
});

it('draws what the plugin says is loaded rather than a list of its own', async () => {
  // ⛔ TASK-136: the panel this replaces rendered eight hardcoded disabled
  // buttons and "No kit yet" while a twelve-pad kit was loaded and playing.
  invoke.mockResolvedValueOnce(kitState());
  await useKit.getState().refresh();

  const { id, lanes, loaded } = useKit.getState();
  expect(id).toBe('trap-default');
  expect(loaded).toBe(true);
  expect(lanes).toHaveLength(ALL_LANES.length);
  expect(lanes.map((l) => l.lane)).toEqual(ALL_LANES);
});

it('separates "not asked yet" from "nothing loaded"', async () => {
  // The panel shows a different thing for each, and it has to: an empty grid
  // before the first reply reads as "this plugin has no kit", which is the
  // untrue readout the whole task is about.
  expect(useKit.getState().loaded).toBe(false);

  invoke.mockResolvedValueOnce({ id: null, lanes: [] });
  await useKit.getState().refresh();
  expect(useKit.getState().loaded).toBe(true);
  expect(useKit.getState().lanes).toEqual([]);
});

it('reports a lane with no voice at all rather than drawing it like the rest', async () => {
  // ⚠ `snap` is in the drum generator's lane list and the shipped kit has
  // never had a pad for it, so it renders silence. Assigning a one-shot is the
  // only way to hear it, and the panel is the only place that can say so.
  invoke.mockResolvedValueOnce(kitState());
  await useKit.getState().refresh();

  const snap = useKit.getState().lanes.find((l) => l.lane === 'snap');
  expect(snap?.shipped).toBe(false);
  expect(snap?.name).toBeNull();
});

it('polls until the dialog closes, then re-reads the kit', async () => {
  vi.useFakeTimers();
  invoke
    .mockResolvedValueOnce(undefined) // one_shot_assign
    .mockResolvedValueOnce({ state: 'running' }) // still browsing
    .mockResolvedValueOnce({ state: 'done', lane: 'melody', name: 'glass.wav' })
    .mockResolvedValueOnce(kitState({ melody: 'glass.wav' }));

  const assigning = useKit.getState().assign('melody');
  await vi.advanceTimersByTimeAsync(0);
  expect(useKit.getState().assigning).toBe('melody');

  await vi.advanceTimersByTimeAsync(500);
  await assigning;

  expect(useKit.getState().assigning).toBeNull();
  expect(useKit.getState().error).toBeNull();
  const melody = useKit.getState().lanes.find((l) => l.lane === 'melody');
  expect(melody?.name).toBe('glass.wav');
  expect(invoke).toHaveBeenCalledWith('one_shot_assign', { lane: 'melody' });
});

it('treats a closed dialog as the ordinary way out, not an error', async () => {
  // ⛔ Reporting a cancel as a failure trains people to ignore the one message
  // that matters — the same rule `export::Status` is built on.
  invoke
    .mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce({ state: 'cancelled' })
    .mockResolvedValueOnce(kitState());

  await useKit.getState().assign('melody');

  expect(useKit.getState().assigning).toBeNull();
  expect(useKit.getState().error).toBeNull();
});

it('shows why a sample was refused', async () => {
  invoke
    .mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce({ state: 'failed', reason: 'that file is silent' })
    .mockResolvedValueOnce(kitState());

  await useKit.getState().assign('melody');

  expect(useKit.getState().error).toBe('that file is silent');
  expect(useKit.getState().assigning).toBeNull();
});

it('adopts an in-flight dialog rather than stranding the producer', async () => {
  // ⛔ The plugin keeps one dialog slot and only this poll drains it. A page
  // that stopped polling — a reloaded webview, an editor torn down and reopened
  // — would otherwise meet "already being chosen" with no dialog anywhere on
  // screen and no way back. Falling through to the poll picks it up.
  invoke
    .mockRejectedValueOnce(
      new Error('a sample is already being chosen — finish that one first'),
    )
    .mockResolvedValueOnce({ state: 'done', lane: 'kick', name: 'my-kick.wav' })
    .mockResolvedValueOnce(kitState({ kick: 'my-kick.wav' }));

  await useKit.getState().assign('kick');

  expect(useKit.getState().error).toBeNull();
  expect(useKit.getState().lanes.find((l) => l.lane === 'kick')?.name).toBe('my-kick.wav');
});

it('refuses a second assignment while one is open', async () => {
  // The plugin refuses it too; this is what stops the page from asking.
  useKit.setState({ assigning: 'melody' });
  await useKit.getState().assign('kick');
  expect(invoke).not.toHaveBeenCalled();
});

it('clears a lane and re-reads what plays it', async () => {
  invoke.mockResolvedValueOnce(undefined).mockResolvedValueOnce(kitState());

  await useKit.getState().clear('melody');

  expect(invoke).toHaveBeenNthCalledWith(1, 'one_shot_clear', { lane: 'melody' });
  expect(useKit.getState().lanes.find((l) => l.lane === 'melody')?.name).toBeNull();
});

it('says so when it cannot read the kit at all', async () => {
  // An empty grid with no explanation is the readout-that-lies failure
  // arriving through the error path.
  invoke.mockRejectedValueOnce(new Error('the bridge is unreachable'));
  await useKit.getState().refresh();

  expect(useKit.getState().error).toBe('the bridge is unreachable');
  expect(useKit.getState().loaded).toBe(true);
});
