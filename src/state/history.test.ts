import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { canRedo, canUndo, useHistory, type Snapshot } from './history';

/**
 * The operation log (FMM-U01).
 *
 * Driven through the store rather than the keyboard: what has to hold is that
 * the baseline is unreachable, that a run of edits to one control is one step,
 * that a generation never merges into the edit before it, and that a fresh edit
 * after an undo abandons the redo branch. None of the four is visible in the DOM.
 */

const BASE: Snapshot = {
  selectedId: null,
  seed: '',
  bars: 4,
  pins: {
    bpm: null,
    keyRoot: null,
    scale: null,
    swing: null,
    timeSigNum: null,
    timeSigDen: null,
  },
  autoSync: true,
  pattern: null,
  mood: null,
  audioEnabled: true,
  mutedLanes: [],
  edited: false,
  song: null,
  songEdited: false,
};

function snap(over: Partial<Snapshot>): Snapshot {
  return { ...BASE, ...over };
}

/** A stand-in for a generated pattern — only its identity matters here. */
function pattern(id: string): Snapshot['pattern'] {
  return { id } as NonNullable<Snapshot['pattern']>;
}

beforeEach(() => {
  vi.useFakeTimers();
  useHistory.setState({ past: [], present: null, future: [] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('the operation log', () => {
  it('records nothing until it is armed', () => {
    // The project restore runs before `arm`, and undoing onto an empty plugin
    // would read as the project having failed to load.
    useHistory.getState().record(snap({ seed: '2024' }));

    expect(useHistory.getState().present).toBeNull();
    expect(canUndo(useHistory.getState())).toBe(false);
  });

  it('cannot step behind the baseline it was armed at', () => {
    const history = useHistory.getState();
    history.arm(snap({ selectedId: 'trap', seed: '2024' }));

    expect(canUndo(useHistory.getState())).toBe(false);
    expect(history.undo()).toBeNull();
  });

  it('steps back and forward through one change', () => {
    const history = useHistory.getState();
    history.arm(BASE);
    history.record(snap({ bars: 8 }));

    expect(canUndo(useHistory.getState())).toBe(true);
    expect(history.undo()?.bars).toBe(4);

    expect(canRedo(useHistory.getState())).toBe(true);
    expect(history.redo()?.bars).toBe(8);
  });

  it('collapses a run of edits to one field into a single step', () => {
    // Typing a six-digit seed is one action to the person typing it. Six undo
    // steps to get back would make the seed box hostile.
    const history = useHistory.getState();
    history.arm(BASE);

    for (const seed of ['2', '20', '202', '2024']) {
      vi.advanceTimersByTime(50);
      history.record(snap({ seed }));
    }

    expect(useHistory.getState().past).toHaveLength(1);
    expect(history.undo()?.seed).toBe('');
  });

  it('starts a new step once the run pauses', () => {
    const history = useHistory.getState();
    history.arm(BASE);

    history.record(snap({ seed: '20' }));
    vi.advanceTimersByTime(5_000);
    history.record(snap({ seed: '2024' }));

    expect(useHistory.getState().past).toHaveLength(2);
    expect(history.undo()?.seed).toBe('20');
  });

  it('does not merge edits to different fields', () => {
    const history = useHistory.getState();
    history.arm(BASE);

    history.record(snap({ seed: '2024' }));
    history.record(snap({ seed: '2024', bars: 8 }));

    expect(useHistory.getState().past).toHaveLength(2);
    expect(history.undo()).toMatchObject({ seed: '2024', bars: 4 });
  });

  it('never merges a generation into the edit before it', () => {
    // ⛔ Two generations back to back are two deliberate acts, however fast the
    // user rerolls. Coalescing them would make the previous beat unreachable.
    const history = useHistory.getState();
    history.arm(BASE);

    history.record(snap({ pattern: pattern('a') }));
    vi.advanceTimersByTime(10);
    history.record(snap({ pattern: pattern('b') }));

    expect(useHistory.getState().past).toHaveLength(2);
    expect(history.undo()?.pattern?.id).toBe('a');
  });

  it('ignores a write that changed nothing', () => {
    const history = useHistory.getState();
    history.arm(BASE);
    history.record(snap({}));

    expect(useHistory.getState().past).toHaveLength(0);
    expect(canUndo(useHistory.getState())).toBe(false);
  });

  it('abandons the redo branch when a new edit lands', () => {
    const history = useHistory.getState();
    history.arm(BASE);
    history.record(snap({ bars: 8 }));
    history.undo();

    expect(canRedo(useHistory.getState())).toBe(true);

    vi.advanceTimersByTime(5_000);
    history.record(snap({ seed: '2024' }));

    expect(canRedo(useHistory.getState())).toBe(false);
    expect(history.redo()).toBeNull();
  });

  it('keeps unlimited depth rather than dropping the oldest step', () => {
    // "Unlimited" is affordable because an entry is five scalars and a shared
    // pattern reference — not a copy of the notes.
    const history = useHistory.getState();
    history.arm(BASE);

    for (let i = 1; i <= 500; i += 1) {
      vi.advanceTimersByTime(5_000);
      history.record(snap({ seed: String(i) }));
    }

    expect(useHistory.getState().past).toHaveLength(500);

    for (let i = 0; i < 500; i += 1) history.undo();
    expect(useHistory.getState().present?.state.seed).toBe('');
  });

  it('steps each lane mute back on its own rather than merging them', () => {
    // ⛔ **Two rules at once, and both had shipped broken.** `mutedLanes` was in
    // `send()` but not in the snapshot, so a mute saved when clicked and was
    // lost when undone — nothing in this file varied the field, so nothing
    // caught it. And a mute is a *discrete act*: coalescing suits a seed being
    // typed toward a value, but muting the kick and then the snare inside the
    // 600 ms window is two intentions, and merging them made "kick muted, snare
    // audible" unreachable by undo.
    const history = useHistory.getState();
    history.arm(BASE);

    history.record(snap({ mutedLanes: ['kick'] }));
    // Well inside COALESCE_MS — the point is that it does not merge anyway.
    vi.advanceTimersByTime(50);
    history.record(snap({ mutedLanes: ['kick', 'snare'] }));

    expect(history.undo()?.mutedLanes).toEqual(['kick']);
    expect(history.undo()?.mutedLanes).toEqual([]);
  });

  it('restores the mute set a redo steps back into', () => {
    const history = useHistory.getState();
    history.arm(BASE);

    vi.advanceTimersByTime(5_000);
    history.record(snap({ mutedLanes: ['openHat'] }));
    expect(history.undo()?.mutedLanes).toEqual([]);
    expect(history.redo()?.mutedLanes).toEqual(['openHat']);
  });
});
