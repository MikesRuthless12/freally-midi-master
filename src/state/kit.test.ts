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
// ⚠ The mock's own idea of an unedited pad, so the fixture below and the
// browser fixture cannot drift apart.
const { untouchedPad } = await import('../lib/ipc-mock');

/**
 * A `kit_state` reply with `assigned` carrying the producer's own samples.
 *
 * ⚠ **`tweaks` on every row, because the real command sends it on every row** —
 * including the lanes nobody has edited, which is what stops the page ever
 * having to invent a default. A fixture that omitted it here would let
 * `setTweaks` be written against a shape the plugin never answers with.
 */
function kitState(assigned: Partial<Record<Lane, string>> = {}) {
  return {
    id: 'trap-default',
    lanes: ALL_LANES.map((lane) => ({
      lane,
      shipped: lane !== 'snap',
      name: assigned[lane] ?? null,
      path: assigned[lane] ? `C:/samples/${assigned[lane]}` : null,
      tweaks: untouchedPad(),
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

/**
 * Per-pad edits (TASK-055A, TASK-164).
 *
 * ⛔ What these are for is the **whole block travelling together**. The plugin
 * clamps and rebuilds on one call, so sending a partial would leave the audio
 * thread holding a state the panel never showed — and the optimistic write is
 * what makes a dragged knob follow the pointer instead of the round trip.
 */
it('sends the whole pad block, not just the field that moved', async () => {
  invoke.mockResolvedValueOnce(kitState());
  await useKit.getState().refresh();
  invoke.mockReset();
  invoke.mockResolvedValueOnce(undefined);

  await useKit.getState().setTweaks('kick', { gainDb: -6 });

  expect(invoke).toHaveBeenCalledWith('pad_tweaks_set', {
    lane: 'kick',
    // ⚠ Every field, with the one change over the plugin's own defaults —
    // never a partial, and never a default this file invented.
    tweaks: { ...untouchedPad(), gainDb: -6 },
  });
});

it('applies a patch over what the plugin last sent rather than over a default', async () => {
  // ⛔ Two edits in a row must accumulate. Spreading over a fresh default would
  // silently undo the first one — a producer sets a decay, then a pan, and the
  // decay quietly goes back to zero.
  invoke.mockResolvedValueOnce(kitState());
  await useKit.getState().refresh();
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);

  await useKit.getState().setTweaks('kick', { gainDb: -6 });
  await useKit.getState().setTweaks('kick', { pan: -1 });

  const row = useKit.getState().lanes.find((l) => l.lane === 'kick');
  expect(row?.tweaks.gainDb).toBe(-6);
  expect(row?.tweaks.pan).toBe(-1);
});

it('moves the knob before the plugin answers, and puts it back if it refuses', async () => {
  // ⛔ The optimistic half is not decoration: these are dragged controls, and a
  // knob that only moves once a round trip completes does not follow the
  // pointer. The rollback is the other half — a control left showing a value
  // the plugin refused is the readout-that-lies failure on a knob, where it is
  // worse than on a label because the producer goes on turning it.
  invoke.mockResolvedValueOnce(kitState());
  await useKit.getState().refresh();
  invoke.mockReset();
  invoke.mockRejectedValueOnce(new Error('that lane has no pad'));

  await useKit.getState().setTweaks('kick', { gainDb: -6 });

  const row = useKit.getState().lanes.find((l) => l.lane === 'kick');
  expect(row?.tweaks.gainDb).toBe(0);
  expect(useKit.getState().error).toBe('that lane has no pad');
});

it('does nothing for a lane the kit has never reported', async () => {
  // ⚠ Rather than inventing a row. The lane list is the plugin's, and a store
  // that answered for a lane it has not been told about would be the second
  // source of truth this file exists to avoid.
  await useKit.getState().setTweaks('kick', { gainDb: -6 });
  expect(invoke).not.toHaveBeenCalled();
});

/**
 * The assignment gesture opens the editor it lands in (TASK-059).
 *
 * ⛔ This lives in the store rather than in `KitPanel`, and the test is here for
 * the same reason: **three** gestures assign a sample — the KIT row's drop, a
 * pad's drop, and the pad grid's "use selected" — and only the first is in that
 * component. A `useState` there could serve one of the three.
 */
it('opens one pad editor at a time and brings its panel on screen', async () => {
  const { useUi } = await import('./ui');
  // Somewhere else entirely, so showing KIT is a visible change.
  useUi.getState().showSection('genres');

  useKit.getState().editPad('kick');
  expect(useKit.getState().editingPad).toBe('kick');
  expect(useUi.getState().sections.kit).toBe(true);

  useKit.getState().editPad('snare');
  expect(useKit.getState().editingPad).toBe('snare');
});

it('does not rearrange the rail when an editor is closed', async () => {
  // ⚠ Only on the way *in*. Closing an editor must not move the panel the
  // producer is looking at out from under them.
  const { useUi } = await import('./ui');
  useKit.getState().editPad('kick');
  useUi.getState().showSection('genres');

  useKit.getState().editPad(null);
  expect(useKit.getState().editingPad).toBeNull();
  expect(useUi.getState().sections.genres).toBe(true);
});
