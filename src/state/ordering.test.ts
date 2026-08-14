import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * What has to happen before what, inside the plugin.
 *
 * ⛔⛔ **Its own file because every assertion here is an ORDERING, and the
 * orderings only exist when `isPlugin()` is true.** `session.test` runs with it
 * false — where `persist` is a no-op and nothing arms the audio thread — so the
 * races these guard cannot be expressed there at all. Two of them:
 *
 * - **A pin has to REACH the plugin before the chips are read back.**
 *   `session_defaults` takes a style id and nothing else: it resolves the model
 *   through whatever the plugin's *saved* session holds for `mood` and `base`.
 *   A refetch issued before the save has landed fills the chips in with the pin
 *   the producer just moved away from — the same wrong readout as never
 *   refetching, reached through more code.
 * - **An import has to switch parts off BEFORE it writes the clips.** Writing
 *   `patterns` fires the subscriber that arms the audio thread, and that reads
 *   `partsOff` at the moment it runs.
 */

const invoke = vi.fn();
vi.mock('../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

// The plugin gate. Without it `persistNow` resolves without sending anything and
// the sequence under test collapses to a single call.
vi.mock('../lib/ipc-plugin', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../lib/ipc-plugin')>()),
  isPlugin: () => true,
}));

const { useSession } = await import('./session');

/** Every command the store sent, in the order it sent them. */
let sent: string[] = [];

/** Lets the pending `save_session_state` finish. */
let releaseSave: () => void = () => {};

beforeEach(() => {
  sent = [];
  releaseSave = () => {};
  invoke.mockReset();
  invoke.mockImplementation((command: string) => {
    sent.push(command);
    if (command === 'save_session_state') {
      // Held open, so "did the read wait?" is a question with an answer rather
      // than a matter of which microtask happened to run first.
      return new Promise((resolve) => {
        releaseSave = () => resolve(null);
      });
    }
    return Promise.resolve(null);
  });
  useSession.setState({ selectedId: 'trap', mood: null, base: null });
});

for (const [name, pin] of [
  ['mood', () => useSession.getState().setMood('dark')],
  ['base', () => useSession.getState().setBase('boom-bap')],
] as const) {
  it(`waits for the plugin to have the ${name} before reading the chips back`, async () => {
    pin();

    // A few microtask turns — enough for any un-awaited `invoke` to have gone
    // out. The read must still not have happened.
    await Promise.resolve();
    await Promise.resolve();
    expect(sent).toContain('save_session_state');
    expect(sent).not.toContain('session_defaults');

    releaseSave();
    await vi.waitFor(() => expect(sent).toContain('session_defaults'));
    // ⚠ And in that order, which is the whole point: `indexOf` rather than a
    // bare `toContain`, because both being present says nothing.
    expect(sent.indexOf('save_session_state')).toBeLessThan(sent.indexOf('session_defaults'));
  });
}

it('still reads the chips back when the save fails', async () => {
  // ⚠ A save that failed leaves the chips showing the previous pin, which is
  // wrong; leaving them showing the *old* value forever with no attempt to
  // refresh is worse, and a rejected promise here would do exactly that.
  invoke.mockImplementation((command: string) => {
    sent.push(command);
    return command === 'save_session_state'
      ? Promise.reject(new Error('the host refused the write'))
      : Promise.resolve(null);
  });

  useSession.getState().setMood('bounce');
  await vi.waitFor(() => expect(sent).toContain('session_defaults'));
});

/**
 * TASK-058H's switch-off, against the arm it has to beat.
 *
 * ⛔⛔ **`armCurrentPattern` reads `partsOff` at the moment it runs**, and what
 * runs it is the subscriber on `patterns`. So switching parts off *after* the
 * clips are written is one microtask too late: the arm has already gone out with
 * the old list, and a chords clip the producer generated earlier is armed and
 * sounding under an import that contains none — which is exactly what the
 * switch-off exists to prevent.
 */
describe('an import switches parts off before it arms anything', () => {
  const clip = (part: string) => ({
    id: `x-${part}`,
    part,
    artistId: '',
    seed: '0',
    songSeed: '0',
    bars: 4,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 0,
    scale: 'naturalMinor',
    lanes: [],
    ppq: 960,
  });

  const split = (part: string) => ({
    part,
    reason: 'percussiveBand',
    notes: 4,
    pattern: clip(part),
  });

  beforeEach(async () => {
    invoke.mockImplementation(() => Promise.resolve(null));
    const { useUi } = await import('./ui');
    useUi.setState({ partsOff: [], activeTab: 'drums' });
  });

  it('never arms a clip the split did not produce', async () => {
    const { useUi } = await import('./ui');
    // The producer made a chords part earlier; the import contains none.
    useSession.setState({ patterns: { chords: clip('chords') as never }, mutedLanes: [] });
    // ⚠ **Cleared after the setup, because the setup arms too** — writing
    // `patterns` is what fires the subscriber, and that is exactly the mechanism
    // under test. What must be empty of chords is the arming the *import* does.
    invoke.mockClear();
    sent.length = 0;

    useSession.getState().importSplit([split('drums') as never]);

    // ⛔ **Every** arm, not just the last: the defect was an extra arm that went
    // out first and was then corrected, and the audio thread had already been
    // handed the chords clip by then.
    const armed = invoke.mock.calls
      .filter(([name]: unknown[]) => name === 'arm_pattern')
      .flatMap(([, args]: unknown[]) => (args as { patterns: { part: string }[] }).patterns);
    expect(armed.map((pattern) => pattern.part)).not.toContain('chords');
    expect(useUi.getState().partsOff).toContain('chords');
  });
});
