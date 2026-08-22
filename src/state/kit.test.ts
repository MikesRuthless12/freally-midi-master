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
      reversed: false,
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

/**
 * The batch import's report (TASK-049).
 *
 * ⛔ **The point is that a partly-refused batch is NOT an error.** The plugin
 * publishes one terminal status carrying a count and a list; the store has to
 * keep the two apart from `error`, or eighteen pads landing would either shout
 * at the producer or throw away the names of the two that did not.
 */
it('reports a batch import per file, and does not call it an error', async () => {
  invoke
    .mockResolvedValueOnce(undefined) // one_shot_add_many
    .mockResolvedValueOnce({
      state: 'imported',
      loaded: 18,
      refused: [
        { name: 'broken.wav', reason: 'that file could not be decoded' },
        {
          name: 'untitled-7.wav',
          reason: 'could not tell from the name which pad this is for',
        },
      ],
    })
    .mockResolvedValueOnce(kitState({ kick: 'Kick 01.wav' }));

  await useKit.getState().addMany();

  const { imported, error } = useKit.getState();
  expect(error).toBeNull();
  expect(imported?.loaded).toBe(18);
  expect(imported?.refused.map((one) => one.name)).toEqual(['broken.wav', 'untitled-7.wav']);
});

it('clears the last import report when the next one starts', async () => {
  // ⚠ The rule `refresh` states about `error`: what the previous import did
  // stopped being true the moment the button was pressed again.
  useKit.setState({ imported: { loaded: 3, refused: [] } });
  invoke
    .mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce({ state: 'cancelled' })
    .mockResolvedValueOnce(kitState());

  await useKit.getState().addMany();
  expect(useKit.getState().imported).toBeNull();
});

it('does not open a second batch dialog while one is running', async () => {
  // The same single-slot rule `assign` follows — the plugin keeps one dialog
  // slot and refusing here is cheaper than being refused across the bridge.
  useKit.setState({ assigning: 'kick' });
  await useKit.getState().addMany();
  expect(invoke).not.toHaveBeenCalled();
});

/**
 * The kit is on the one undo stack (TASK-050A).
 *
 * ⛔ **What has to hold is the reference stability, not just the value.**
 * `history.changed` compares snapshot fields by reference and `refresh` runs
 * after every gesture in the app — so a fresh map each time would report the
 * kit as changed on every snapshot, record a step for every seed keystroke, and
 * stop anything ever coalescing.
 */
it('keeps the same kit object when nothing on the pads moved', async () => {
  invoke.mockResolvedValue(kitState({ kick: 'Kick 01.wav' }));

  await useKit.getState().refresh();
  const first = useKit.getState().oneShots;
  await useKit.getState().refresh();

  expect(useKit.getState().oneShots).toBe(first);
  expect(first).toEqual({ kick: { path: 'C:/samples/Kick 01.wav', reversed: false } });
});

it('replaces the kit object when a pad changes', async () => {
  invoke.mockResolvedValueOnce(kitState({ kick: 'Kick 01.wav' }));
  await useKit.getState().refresh();
  const first = useKit.getState().oneShots;

  invoke.mockResolvedValueOnce(kitState({ kick: 'Kick 02.wav' }));
  await useKit.getState().refresh();

  expect(useKit.getState().oneShots).not.toBe(first);
  expect(useKit.getState().oneShots).toEqual({
    kick: { path: 'C:/samples/Kick 02.wav', reversed: false },
  });
});

it('reports only the lanes holding the producer own samples', async () => {
  // ⚠ A shipped pad has no path and is absent — which is what the plugin reads
  // as "put the built-in sound back on this one".
  invoke.mockResolvedValueOnce(kitState({ snare: 'clap.wav' }));
  await useKit.getState().refresh();
  expect(useKit.getState().oneShots).toEqual({
    snare: { path: 'C:/samples/clap.wav', reversed: false },
  });
});

it('says what could not be put back, and stays quiet when everything was', async () => {
  // ⛔ A clean undo is silent — the same rule `oneshot::restore` states for a
  // reopened project. A sample that has moved since is still reported.
  invoke
    .mockResolvedValueOnce({ state: 'restored', refused: [] })
    .mockResolvedValueOnce(kitState());
  await useKit.getState().awaitLoader();
  expect(useKit.getState().error).toBeNull();
  expect(useKit.getState().imported).toBeNull();

  invoke
    .mockResolvedValueOnce({
      state: 'restored',
      refused: [{ name: 'gone.wav', reason: 'no such file' }],
    })
    .mockResolvedValueOnce(kitState());
  await useKit.getState().awaitLoader();
  expect(useKit.getState().error).toContain('gone.wav');
});

it('carries which way round a pad plays, so an undo cannot un-reverse it', async () => {
  // ⛔ `oneshot::load` bakes a reversal into the buffer, so a path alone does
  // not describe the sound. A snapshot that dropped the flag would restore the
  // file playing forwards and `apply` would then persist that loss — the
  // producer's `Ctrl`+← undone by a Ctrl+Z that was about something else.
  const backwards = kitState({ kick: 'Kick 01.wav' });
  backwards.lanes = backwards.lanes.map((row) =>
    row.lane === 'kick' ? { ...row, reversed: true } : row,
  );
  invoke.mockResolvedValueOnce(backwards);
  await useKit.getState().refresh();

  expect(useKit.getState().oneShots).toEqual({
    kick: { path: 'C:/samples/Kick 01.wav', reversed: true },
  });
});

it('treats the same file played backwards as a different kit', async () => {
  // Reference stability must not paper over the direction: if it did, an undo
  // across a reverse would compare equal and skip the restore entirely.
  invoke.mockResolvedValueOnce(kitState({ kick: 'Kick 01.wav' }));
  await useKit.getState().refresh();
  const forwards = useKit.getState().oneShots;

  const flipped = kitState({ kick: 'Kick 01.wav' });
  flipped.lanes = flipped.lanes.map((row) =>
    row.lane === 'kick' ? { ...row, reversed: true } : row,
  );
  invoke.mockResolvedValueOnce(flipped);
  await useKit.getState().refresh();

  expect(useKit.getState().oneShots).not.toBe(forwards);
});

it('records one undo step for a sample dropped from the browser', async () => {
  // ⛔ Drag and drop is the primary way a sample reaches a pad, and it used to
  // record nothing — so the next Ctrl+Z reverted the drop silently along with
  // whatever edit it was actually about. Five gestures do this; they all go
  // through `drop` now.
  const { useExplorer } = await import('./explorer');
  const dropOn = vi.fn().mockResolvedValue(true);
  useExplorer.setState({ dropOn });
  invoke.mockResolvedValue(kitState({ kick: 'Kick 01.wav' }));

  const landed = await useKit.getState().drop('kick', 'C:/samples/Kick 01.wav');

  expect(landed).toBe(true);
  expect(dropOn).toHaveBeenCalledWith('kick', 'C:/samples/Kick 01.wav', false);
  expect(useKit.getState().oneShots).toEqual({
    kick: { path: 'C:/samples/Kick 01.wav', reversed: false },
  });
});

it('does not record a step for a drop the plugin refused', async () => {
  const { useExplorer } = await import('./explorer');
  useExplorer.setState({ dropOn: vi.fn().mockResolvedValue(false) });
  invoke.mockResolvedValue(kitState());

  expect(await useKit.getState().drop('kick', 'C:/elsewhere/kick.wav')).toBe(false);
});

/**
 * Ctrl+Z held down (TASK-050A).
 *
 * ⛔ Restoring a kit is asynchronous while the session fields it travels with
 * are restored synchronously, and the plugin keeps one loader slot — so a
 * second restore issued while the first was still decoding used to be refused
 * with *"already"* and surface as an error over a kit left a step behind. The
 * newest wanted kit is parked instead, and it is the one that lands.
 */
it('sends one restore at a time and lands on the last kit asked for', async () => {
  const { restoreKit } = await import('./kit');
  const sent: unknown[] = [];

  invoke.mockImplementation((command: string, args?: unknown) => {
    if (command === 'one_shot_set_all') {
      sent.push(args);
      return Promise.resolve(undefined);
    }
    if (command === 'one_shot_status')
      return Promise.resolve({ state: 'restored', refused: [] });
    return Promise.resolve(kitState());
  });

  const first = restoreKit({ oneShots: { kick: { path: 'C:/a.wav', reversed: false } } });
  const second = restoreKit({ oneShots: { snare: { path: 'C:/b.wav', reversed: true } } });
  await Promise.all([first, second]);

  // ⛔ The second call parked its kit rather than opening a second restore, so
  // the last one asked for is the one that reached the plugin.
  const last = sent[sent.length - 1] as { lanes: [string, string, boolean][] };
  expect(last.lanes).toEqual([['snare', 'C:/b.wav', true]]);
});

it('does not send a restore for the kit already loaded', async () => {
  // The guard that makes undo affordable: one seed keystroke carries the kit in
  // its snapshot, and re-decoding a producer's samples to arrive where they
  // already are would cut every sounding voice.
  const { restoreKit } = await import('./kit');
  invoke.mockResolvedValue(kitState({ kick: 'Kick 01.wav' }));
  await useKit.getState().refresh();
  invoke.mockClear();

  await restoreKit({
    oneShots: { kick: { path: 'C:/samples/Kick 01.wav', reversed: false } },
  });
  expect(invoke).not.toHaveBeenCalledWith('one_shot_set_all', expect.anything());
});

it('lets a redo cancel the undo that is still arriving', async () => {
  // ⛔ While a restore is in flight the store still holds the kit it is
  // *leaving*, so comparing against the store alone answered about a state
  // already being replaced: undo to B then immediately redo to A, and "A is
  // loaded" is true and useless — B is what is arriving, and it would have
  // landed on top of the redo.
  const { restoreKit } = await import('./kit');
  const sent: string[] = [];
  // ⚠ **A STATEFUL fake, because the real plugin is stateful.** A `kit_state`
  // that answered the same fixture whatever was applied would report the kit as
  // never changing, and every restore after the first would correctly be
  // skipped as a no-op — a green test watching nothing.
  let applied: Record<string, string> = {};

  invoke.mockImplementation((command: string, args?: unknown) => {
    if (command === 'one_shot_set_all') {
      const { lanes } = args as { lanes: [string, string, boolean][] };
      sent.push(lanes.map(([lane]) => lane).join(',') || '(empty)');
      applied = Object.fromEntries(lanes.map(([lane, path]) => [lane, path]));
      return Promise.resolve(undefined);
    }
    if (command === 'one_shot_status')
      return Promise.resolve({ state: 'restored', refused: [] });
    return Promise.resolve(kitState(applied as Partial<Record<Lane, string>>));
  });

  const undo = restoreKit({ oneShots: { snare: { path: 'C:/b.wav', reversed: false } } });
  // The store still reads as the empty kit here — the restore has not landed.
  const redo = restoreKit({ oneShots: {} });
  await Promise.all([undo, redo]);

  // ⚠ The undo may or may not reach the plugin first depending on scheduling —
  // what must hold is that the kit is not left holding it.
  expect(sent[sent.length - 1]).toBe('(empty)');
  expect(useKit.getState().oneShots).toEqual({});
});

it('does not park a lane in `assigning` while a batch import runs', async () => {
  // ⛔ The second cut set `assigning: 'kick'` as a stand-in for "a dialog is
  // open", on the claim that every reader only asks whether it is null. That
  // was false: `KitPanel` compares it to a lane to draw "choosing…" over that
  // row's sample name, so pressing Add samples… made the kick row stop naming
  // its own sample. A batch belongs to no lane.
  let openWhileRunning: { assigning: unknown; importing: boolean } | null = null;
  invoke.mockImplementation((command: string) => {
    if (command === 'one_shot_add_many') {
      const { assigning, importing } = useKit.getState();
      openWhileRunning = { assigning, importing };
      return Promise.resolve(undefined);
    }
    if (command === 'one_shot_status') return Promise.resolve({ state: 'cancelled' });
    return Promise.resolve(kitState());
  });

  await useKit.getState().addMany();

  expect(openWhileRunning).toEqual({ assigning: null, importing: true });
  expect(useKit.getState().importing).toBe(false);
});

it('records one undo step for a saved kit being loaded', async () => {
  // ⛔ `SavedKits` invoked `kits_load` and awaited the loader directly, and
  // recorded nothing — so the next Ctrl+Z, about anything at all, restored a
  // snapshot still naming the kit from before the load and unloaded it.
  invoke.mockImplementation((command: string) => {
    if (command === 'kits_load') return Promise.resolve(undefined);
    if (command === 'one_shot_status') return Promise.resolve({ state: 'idle' });
    return Promise.resolve(kitState({ kick: 'Kick 01.wav' }));
  });

  await useKit.getState().loadSaved('my-trap-kit');

  expect(invoke).toHaveBeenCalledWith('kits_load', { id: 'my-trap-kit' });
  expect(useKit.getState().oneShots).toEqual({
    kick: { path: 'C:/samples/Kick 01.wav', reversed: false },
  });
});
