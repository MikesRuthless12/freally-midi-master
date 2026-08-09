import { beforeEach, expect, it, vi } from 'vitest';

import type { Pattern } from '../lib/ipc-types';

/**
 * The drag-out gesture (TASK-063C).
 *
 * ⛔ **What these exist for is the race.** Rendering takes long enough that the
 * producer can let go before the files are written, and starting the OS drag
 * then makes `DoDragDrop` see no button on its first question, answer "dropped",
 * and put the loop wherever the cursor happened to be — a file landing on a
 * random track, which is the worst possible reading of "the drag did nothing".
 * Everything else here is a pass-through to the bridge.
 */

const invoke = vi.fn();
vi.mock('../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

const { useDrag } = await import('./drag');
type Gesture = import('./drag').Gesture;

const pattern = {
  id: 't',
  part: 'drums',
  artistId: 'trap',
  seed: '7',
  bars: 4,
  bpm: 140,
  timeSigNum: 4,
  timeSigDen: 4,
  keyRoot: 1,
  scale: 'natural_minor',
  lanes: [{ lane: 'kick', notes: [] }],
  ppq: 960,
  mood: null,
  loopRegion: null,
  clipRegion: null,
} as unknown as Pattern;

const subject = { kind: 'patterns' as const, patterns: [pattern] };

/** A gesture in whatever state the test needs. */
function gesture(held: boolean, moved: boolean): Gesture {
  return { held, moved, preview: null };
}

/** Answer each command with whatever the table says, defaulting to `undefined`. */
function answers(table: Record<string, unknown>) {
  invoke.mockImplementation((command: string) => {
    if (command in table) {
      const value = table[command];
      return value instanceof Error ? Promise.reject(value) : Promise.resolve(value);
    }
    return Promise.resolve(undefined);
  });
}

const commands = () => invoke.mock.calls.map(([command]) => command as string);
const argsOf = (command: string) =>
  invoke.mock.calls.find(([name]) => name === command)?.[1] as
    Record<string, unknown> | undefined;

beforeEach(() => {
  invoke.mockReset();
  useDrag.setState({ canDrag: true, state: 'idle', message: null });
});

it('asks the plugin whether a drag is possible rather than guessing from the platform', async () => {
  // ⛔ The day a macOS build ships without `NSFilePromiseProvider`, a page that
  // inferred this from the OS name would offer a handle that drops nothing.
  answers({ drag_supported: { supported: false } });
  await useDrag.getState().loadCapability();
  expect(useDrag.getState().canDrag).toBe(false);
});

it('offers nothing at all when the plugin says it cannot drag', async () => {
  useDrag.setState({ canDrag: false });
  await useDrag.getState().begin(subject, 'midi', gesture(true, true));
  expect(commands()).toEqual([]);
});

it('cancels instead of dropping when the producer lets go before the files exist', async () => {
  // ⛔⛔ The one that matters. `drag_start` here would drop a loop onto whatever
  // the cursor was over.
  answers({ drag_status: { state: 'preparing' } });
  await useDrag.getState().begin(subject, 'midi', gesture(false, true));

  expect(commands()).toContain('drag_cancel');
  expect(commands()).not.toContain('drag_start');
  expect(useDrag.getState().state).toBe('idle');
});

it('waits for the pointer to travel before it starts a drag', async () => {
  // A press that never moved is an ordinary click on the chip.
  answers({ drag_status: { state: 'ready' } });
  await useDrag.getState().begin(subject, 'midi', gesture(false, false));

  expect(commands()).not.toContain('drag_start');
  expect(commands()).toContain('drag_cancel');
});

it('starts the drag once the bytes are written and the hand has moved', async () => {
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });
  await useDrag.getState().begin(subject, 'midi', gesture(true, true));

  expect(commands()).toContain('drag_start');
  // ⚠ Back to idle rather than to a "done" that lingers: a drag ends when the
  // producer lets go, and a message about it would be a toast for a gesture they
  // watched happen.
  expect(useDrag.getState().state).toBe('idle');
});

it('stops polling once the files are ready and only the hand is missing', async () => {
  // ⚠ A producer who presses and holds without moving used to send a round trip
  // every 40 ms — 250 of them before the timeout — to the thread the DAW draws
  // its editor from, each re-learning a fact that cannot change.
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });
  const live = gesture(true, false);
  const running = useDrag.getState().begin(subject, 'midi', live);
  await new Promise((done) => setTimeout(done, 250));
  const polls = commands().filter((command) => command === 'drag_status').length;
  live.moved = true;
  await running;

  expect(polls).toBe(1);
  expect(commands()).toContain('drag_start');
});

it('carries the drag image on drag_start, not on the press', async () => {
  // ⛔ ~96 KB of pixels through JSON on every click is what this avoids.
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });
  const preview = { width: 2, height: 2, rgba: 'AAAA' };
  await useDrag.getState().begin(subject, 'midi', { held: true, moved: true, preview });

  expect(argsOf('drag_prepare')).not.toHaveProperty('preview');
  expect(argsOf('drag_start')).toEqual({ preview });
});

it('names the lane instead of slicing the pattern for the plugin', async () => {
  // ⛔ The page used to send a one-lane copy and ask for "split by lane", which
  // put "what a lane stem is" on both sides of the bridge.
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });
  await useDrag
    .getState()
    .begin(
      { kind: 'patterns', patterns: [pattern], lane: 'closedHat' },
      'audio',
      gesture(true, true),
    );

  expect(argsOf('drag_prepare')).toMatchObject({
    lane: 'closedHat',
    audio: true,
    patterns: [pattern],
  });
});

it('sends the whole arrangement as a song rather than as patterns', async () => {
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });
  const song = { id: 's', sections: [] } as never;
  await useDrag.getState().begin({ kind: 'song', song }, 'midi', gesture(true, true));

  expect(argsOf('drag_prepare')).toMatchObject({ song, patterns: [], audio: false });
});

it('says why it failed rather than leaving the handle looking armed', async () => {
  answers({ drag_status: { state: 'failed', reason: 'there is nothing here to drag' } });
  await useDrag.getState().begin(subject, 'midi', gesture(true, true));

  expect(useDrag.getState().state).toBe('failed');
  expect(useDrag.getState().message).toBe('there is nothing here to drag');
  expect(commands()).not.toContain('drag_start');
});

it('refuses a second gesture while one is still in flight', async () => {
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });
  useDrag.setState({ state: 'dragging' });
  await useDrag.getState().begin(subject, 'midi', gesture(true, true));
  expect(commands()).toEqual([]);
});

it('a release during the status round trip still cancels rather than dropping', async () => {
  // ⛔⛔ The gap the review found. `held` was read at the top of the loop, then
  // the code awaited a round trip to the editor thread — which is also serving
  // the host poll and the export poll — and broke out on `ready` without ever
  // looking at `held` again. A producer who let go during that await had their
  // loop dropped wherever the cursor happened to be.
  const live = gesture(true, true);
  invoke.mockImplementation((command: string) => {
    if (command === 'drag_status') {
      // The release lands while this request is in flight.
      live.held = false;
      return Promise.resolve({ state: 'ready' });
    }
    return Promise.resolve(command === 'drag_start' ? 'copied' : undefined);
  });

  await useDrag.getState().begin(subject, 'midi', live);

  expect(commands()).not.toContain('drag_start');
  expect(commands()).toContain('drag_cancel');
  expect(useDrag.getState().state).toBe('idle');
});

it('a press after a failure retries instead of doing nothing', async () => {
  // ⛔ `begin` refused every state but `idle`, so the first press after any
  // failure was swallowed — and the release then wiped the message too, so the
  // producer saw a chip that did nothing and no explanation.
  useDrag.setState({ state: 'failed', message: 'the preview kit did not load' });
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });

  await useDrag.getState().begin(subject, 'midi', gesture(true, true));

  expect(commands()).toContain('drag_prepare');
  expect(commands()).toContain('drag_start');
});

it('letting go does not erase the error before it can be read', async () => {
  // The message renders only while the state is `failed`, and a release follows
  // the press by a couple of hundred milliseconds.
  useDrag.setState({ state: 'failed', message: 'the preview kit did not load' });
  useDrag.getState().abandon();

  expect(useDrag.getState().state).toBe('failed');
  expect(useDrag.getState().message).toBe('the preview kit did not load');
});

it('stops polling when the plugin says the render was disowned', async () => {
  // A newer press took the slot; carrying on would only run to the 10s timeout
  // and report a failure for a drag that was superseded deliberately.
  answers({ drag_status: { state: 'idle' } });
  await useDrag.getState().begin(subject, 'midi', gesture(true, true));

  expect(useDrag.getState().state).toBe('idle');
  expect(commands()).not.toContain('drag_start');
});

it('two chips do not share one gesture', async () => {
  // ⛔ Module-level gesture flags made a second press clear the first press's
  // "still held" — reachable with touch, which the chips deliberately enable.
  answers({ drag_status: { state: 'ready' }, drag_start: 'copied' });
  const first = gesture(true, true);
  const second = gesture(false, false);

  const running = useDrag.getState().begin(subject, 'midi', first);
  // A second chip is pressed and released while the first is still going.
  second.held = false;
  await running;

  expect(commands()).toContain('drag_start');
  expect(first.held).toBe(true);
});
