import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

const { PadEditor } = await import('./PadEditor');
const { envelopePath } = await import('./padEnvelope');
const { useKit } = await import('../../state/kit');
type KitLane = import('../../state/kit').KitLane;
const { untouchedPad } = await import('../../lib/ipc-mock');

/**
 * The per-pad sound editor (TASK-055A, TASK-164).
 *
 * ⛔ **Driven through the component with the store set directly**, the shape
 * `KitPanel.test.tsx` uses — the store has its own file and was never the thing
 * that was wrong here.
 */
function row(overrides: Partial<ReturnType<typeof plainRow>> = {}) {
  return { ...plainRow(), ...overrides };
}

function plainRow(): KitLane {
  return {
    lane: 'kick',
    shipped: true,
    name: null,
    path: null,
    tweaks: untouchedPad(),
    reversed: false,
    // ⚠ Typed as the row rather than inferred: `root: null` on its own narrows
    // to `null`, so a case that supplies a measured one stops compiling.
    root: null,
    holds: null,
  };
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  useKit.setState({ lanes: [plainRow()], loaded: true, assigning: null, error: null });
});

afterEach(cleanup);

describe('the envelope drawing', () => {
  it('is a flat line at full level when nothing has been set', () => {
    // ⛔ The identity envelope has to *look* like no envelope. A default that
    // drew a curve would tell every producer their untouched kick has been
    // shaped, which is the readout-that-lies failure before a control is even
    // touched — and the audio thread skips this case entirely, so the graph
    // would be describing arithmetic that never runs.
    const path = envelopePath({ attackMs: 0, decayMs: 0, sustainDb: 0, releaseMs: 0 });
    // Attack at x=0 straight to the top, and the sustain leg stays at the top.
    expect(path).toContain('M0,96');
    expect(path).toContain('L0.0,0');
  });

  it('drops the sustain leg as the sustain level falls', () => {
    const loud = envelopePath({ attackMs: 0, decayMs: 100, sustainDb: 0, releaseMs: 0 });
    const quiet = envelopePath({ attackMs: 0, decayMs: 100, sustainDb: -36, releaseMs: 0 });
    expect(loud).not.toEqual(quiet);
    // ⚠ Linear in dB, not in amplitude: −36 of a −60 floor is 40% of the way
    // up, which is where the handle's own readout says it is. In amplitude it
    // would sit on the floor and the drag would stop matching the number.
    expect(quiet).toContain(',57.6');
  });

  it('stops travelling past the drawing span instead of rescaling', () => {
    // ⚠ A graph that rescaled as you dragged would move the handle you are
    // holding away from the pointer. The value is still honoured — the sampler
    // reads milliseconds, not this path.
    const long = envelopePath({ attackMs: 2_000, decayMs: 0, sustainDb: 0, releaseMs: 0 });
    const longer = envelopePath({ attackMs: 30_000, decayMs: 0, sustainDb: 0, releaseMs: 0 });
    expect(long).toEqual(longer);
  });
});

describe('the controls', () => {
  it('sends the whole block on one command when a dial moves', async () => {
    render(<PadEditor entry={row()} onClose={() => {}} />);

    fireEvent.change(screen.getByLabelText('Volume'), { target: { value: '-6' } });
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('pad_tweaks_set', {
        lane: 'kick',
        tweaks: { ...untouchedPad(), gainDb: -6 },
      }),
    );
  });

  it('resets every control in one command rather than ten', async () => {
    // ⛔ Field by field would rebuild the kit ten times and let the audio thread
    // hear nine states that were never on screen.
    const edited = row({
      tweaks: {
        ...untouchedPad(),
        gainDb: -6,
        pan: -1,
        semis: 5,
        trimStart: 0.4,
        adsr: { attackMs: 10, decayMs: 195, sustainDb: -36, releaseMs: 500 },
      },
    });
    // ⛔ **The store's row has to carry the edits too, not just the prop.**
    // `setTweaks` merges the patch over what the STORE holds — so with the store
    // left at `plainRow()` the merge base was already untouched and
    // `edit({})` would have passed this assertion.
    useKit.setState({ lanes: [edited] });
    render(<PadEditor entry={edited} onClose={() => {}} />);

    fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    expect(invoke).toHaveBeenCalledWith('pad_tweaks_set', {
      lane: 'kick',
      tweaks: untouchedPad(),
    });
  });

  it('closes the trim window rather than letting it invert', async () => {
    // ⛔ The plugin clamps an inverted slice — it is the shape that once aborted
    // the host — but a slider that lets you drag past and then silently snaps
    // back is a control fighting its own user.
    const trimmed = row({ tweaks: { ...untouchedPad(), trimEnd: 0.3 } });
    render(<PadEditor entry={trimmed} onClose={() => {}} />);

    fireEvent.change(screen.getByLabelText('Start'), { target: { value: '0.8' } });
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled());
    const sent = invoke.mock.calls[0]?.[1] as {
      tweaks: { trimStart: number; trimEnd: number };
    };
    expect(sent.tweaks.trimStart).toBe(0.8);
    expect(sent.tweaks.trimEnd).toBe(0.8);
  });

  it('says why there is no waveform rather than drawing a stranger', async () => {
    // ⛔ A shipped pad's audio is compiled into the binary and has no path.
    // Drawing a generic waveform would be a picture of a sample that is not the
    // one on the pad.
    render(<PadEditor entry={row({ path: null })} onClose={() => {}} />);

    expect(screen.getByText(/built-in sample/)).toBeTruthy();
    expect(invoke).not.toHaveBeenCalledWith('explorer_waveform', expect.anything());
  });

  it('draws the producer’s own sample when there is a file to read', async () => {
    invoke.mockResolvedValue({ peaks: [[-0.5, 0.5] as [number, number]] });
    render(<PadEditor entry={row({ path: 'C:/s/my-kick.wav' })} onClose={() => {}} />);

    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('explorer_waveform', { path: 'C:/s/my-kick.wav' }),
    );
    // ⚠ Waited for, not asserted straight after the call: the peaks land in
    // state a tick later, and asserting on the frame the request was *made*
    // would pass whether or not they were ever drawn.
    await vi.waitFor(() => expect(screen.queryByText(/built-in sample/)).toBeNull());
  });

  it('still edits a pad whose waveform could not be read', async () => {
    // ⚠ The store's `error` slot is for a failed *edit*. A sample whose picture
    // could not be drawn edits perfectly well, and surfacing that failure there
    // would put an alarm under a working control.
    invoke.mockRejectedValueOnce(new Error('outside your sample library'));
    render(<PadEditor entry={row({ path: 'C:/elsewhere/kick.wav' })} onClose={() => {}} />);

    await vi.waitFor(() => expect(screen.getByText(/built-in sample/)).toBeTruthy());
    expect(useKit.getState().error).toBeNull();
  });

  it('closes on Escape', () => {
    const onClose = vi.fn();
    render(<PadEditor entry={row()} onClose={onClose} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });
});

describe('a pad that plays backwards', () => {
  it('draws the waveform the way the trim handles cut it', async () => {
    // ⛔⛔ **`oneshot::load` flips the buffer at decode**, so the trim window is
    // measured against the *reversed* audio — while `explorer_waveform` reads
    // the file off disk *forwards*. Without the mirror, dragging Start to 0.5
    // shaded the left half of the picture and the engine cut the right half of
    // what was drawn: a control whose own illustration contradicts it.
    const peaks: [number, number][] = [
      [0, 0.1],
      [0, 0.9],
    ];
    invoke.mockResolvedValue({ peaks });

    const forwards = render(
      <PadEditor entry={row({ path: 'C:/s/crash.wav' })} onClose={() => {}} />,
    );
    await vi.waitFor(() =>
      expect(document.querySelector('.pad-editor__wave-fill')).toBeTruthy(),
    );
    const drawnForwards = document.querySelector('.pad-editor__wave-fill')?.getAttribute('d');
    forwards.unmount();

    render(
      <PadEditor entry={row({ path: 'C:/s/crash.wav', reversed: true })} onClose={() => {}} />,
    );
    await vi.waitFor(() =>
      expect(document.querySelector('.pad-editor__wave-fill')).toBeTruthy(),
    );
    const drawnBackwards = document.querySelector('.pad-editor__wave-fill')?.getAttribute('d');

    expect(drawnForwards).toBeTruthy();
    expect(drawnBackwards).not.toEqual(drawnForwards);
  });
});

/**
 * The detected root, surfaced beside the dials that correct it (TASK-052).
 *
 * ⛔ What matters is that the three `null` cases stay silent. A drum lane, a
 * shipped pad and a sample with no clear pitch all report nothing, and printing
 * a note for any of them would be a reading of a measurement never taken.
 */
describe('the detected root', () => {
  it('names the note and how far off it is', () => {
    render(
      <PadEditor
        entry={row({ lane: 'melody', root: { note: 45, cents: 12, clarity: 0.95 } })}
        onClose={() => {}}
      />,
    );
    // MIDI 45 is A2 in the notation the piano roll's gutter already uses.
    expect(document.querySelector('.pad-editor__root')?.textContent).toContain('A2');
    expect(document.querySelector('.pad-editor__root')?.textContent).toContain('+12');
  });

  it('says when it is unsure rather than printing a confidence number', () => {
    // 0.61 means nothing to a producer; "unsure" does. Below `MIN_CLARITY` the
    // plugin sends nothing at all, so this band is the only place doubt shows.
    render(
      <PadEditor
        entry={row({ lane: 'bass', root: { note: 36, cents: -3, clarity: 0.62 } })}
        onClose={() => {}}
      />,
    );
    const said = document.querySelector('.pad-editor__root')?.textContent ?? '';
    expect(said).toContain('C2');
    expect(said).toMatch(/unsure/i);
  });

  it('is confident quietly', () => {
    render(
      <PadEditor
        entry={row({ lane: 'bass', root: { note: 36, cents: 0, clarity: 0.99 } })}
        onClose={() => {}}
      />,
    );
    expect(document.querySelector('.pad-editor__root')?.textContent).not.toMatch(/unsure/i);
  });

  it('says nothing at all for a pad with no measured root', () => {
    render(<PadEditor entry={row()} onClose={() => {}} />);
    expect(document.querySelector('.pad-editor__root')).toBeNull();
  });
});

/**
 * A sample that cannot be held says so (TASK-053A).
 *
 * ⛔ The task's own clause: *"the editor **says so** rather than silently
 * shortening the note"*. Only the `false` case earns a sentence — `true` is
 * what a producer already expects, and `null` is a lane where holding a note
 * means nothing.
 */
describe('holding a note', () => {
  it('says when a long note will end where the file does', () => {
    render(<PadEditor entry={row({ lane: 'chords', holds: false })} onClose={() => {}} />);
    expect(screen.getByText(/steady part/i)).toBeTruthy();
  });

  it('says nothing when the sample can be held', () => {
    render(<PadEditor entry={row({ lane: 'chords', holds: true })} onClose={() => {}} />);
    expect(screen.queryByText(/steady part/i)).toBeNull();
  });

  it('says nothing on a lane where holding means nothing', () => {
    render(<PadEditor entry={row({ holds: null })} onClose={() => {}} />);
    expect(screen.queryByText(/steady part/i)).toBeNull();
  });
});

/**
 * The envelope's handles (TASK-055).
 *
 * ⛔ The geometry is asserted through `envelopePoints` rather than by faking a
 * pointer drag: jsdom gives every element a zero-sized bounding box, so a drag
 * would divide by zero and prove nothing. What a component test can hold is
 * that a handle sits on the curve it moves and that a double-click zeroes its
 * own stage and nothing else.
 */
describe('the envelope handles', () => {
  const shaped = {
    ...untouchedPad(),
    adsr: { attackMs: 200, decayMs: 400, sustainDb: -12, releaseMs: 600 },
  };

  it('draws one grip per stage, on the line it moves', async () => {
    const { envelopePoints } = await import('./padEnvelope');
    render(<PadEditor entry={row({ tweaks: shaped })} onClose={() => {}} />);

    const at = envelopePoints(shaped.adsr);
    for (const [key, point] of Object.entries(at)) {
      const grip = document.querySelector(`.pad-editor__env-grip[data-stage="${key}"]`);
      expect(grip).toBeTruthy();
      expect(Number(grip?.getAttribute('cx'))).toBeCloseTo(point.x, 5);
      expect(Number(grip?.getAttribute('cy'))).toBeCloseTo(point.y, 5);
    }
  });

  it('zeroes only its own stage on a double-click', async () => {
    // ⛔ The header's Reset clears everything; this is what a producer reaches
    // for when three stages are right and the fourth is not.
    useKit.setState({ lanes: [row({ tweaks: shaped })] });
    render(<PadEditor entry={row({ tweaks: shaped })} onClose={() => {}} />);

    const decay = document.querySelector('.pad-editor__env-grip[data-stage="d"]');
    fireEvent.doubleClick(decay!);
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled());

    const sent = invoke.mock.calls[0]?.[1] as { tweaks: { adsr: typeof shaped.adsr } };
    expect(sent.tweaks.adsr).toEqual({ ...shaped.adsr, decayMs: 0 });
  });

  it('maps a height back to the level that drew it', async () => {
    // The inverse the vertical drag depends on: a handle dragged to a point and
    // read back has to land where it was put.
    const { dbOf, envelopePoints: points } = await import('./padEnvelope');
    for (const db of [0, -6, -12, -36, -60]) {
      const y = points({ attackMs: 0, decayMs: 0, sustainDb: db, releaseMs: 0 }).s.y;
      expect(dbOf(y)).toBeCloseTo(db, 5);
    }
  });

  it('maps a distance back to the duration that drew it', async () => {
    const { msOf, envelopePoints: points } = await import('./padEnvelope');
    for (const ms of [0, 50, 500, 2000]) {
      const x = points({ attackMs: ms, decayMs: 0, sustainDb: 0, releaseMs: 0 }).a.x;
      expect(msOf(x)).toBeCloseTo(ms, 5);
    }
  });
});

/**
 * A and B (TASK-055).
 *
 * ⛔ B is a scratchpad, not a saved value: nothing here reaches the plugin
 * until Keep is pressed, and the graph is what shows the difference.
 */
describe('the A/B toggle', () => {
  const shaped = {
    ...untouchedPad(),
    adsr: { attackMs: 200, decayMs: 400, sustainDb: -12, releaseMs: 600 },
  };

  it('cannot show or keep a B that was never stored', () => {
    render(<PadEditor entry={row()} onClose={() => {}} />);
    expect(screen.getByRole('button', { name: /Show B/i })).toHaveProperty('disabled', true);
    expect(screen.getByRole('button', { name: /Keep B/i })).toHaveProperty('disabled', true);
  });

  it('draws B while comparing and sends nothing until Keep', async () => {
    useKit.setState({ lanes: [row({ tweaks: shaped })] });
    const { rerender } = render(
      <PadEditor entry={row({ tweaks: shaped })} onClose={() => {}} />,
    );

    fireEvent.click(screen.getByRole('button', { name: /Store B/i }));
    // A different A to compare against.
    const changed = { ...shaped, adsr: { ...shaped.adsr, attackMs: 20 } };
    useKit.setState({ lanes: [row({ tweaks: changed })] });
    rerender(<PadEditor entry={row({ tweaks: changed })} onClose={() => {}} />);

    const { envelopePath } = await import('./padEnvelope');
    fireEvent.click(screen.getByRole('button', { name: /Show B/i }));
    expect(document.querySelector('.pad-editor__env-line')?.getAttribute('d')).toBe(
      envelopePath(shaped.adsr),
    );
    // Storing and comparing are local: nothing has crossed the bridge.
    expect(invoke).not.toHaveBeenCalledWith('pad_tweaks_set', expect.anything());

    // ⚠ The handles are hidden while B is showing — a handle that moved the
    // value being compared against would make the comparison meaningless.
    expect(document.querySelector('.pad-editor__env-grip')).toBeNull();
  });

  it('keeps B by sending it as the pad’s own shape', async () => {
    useKit.setState({ lanes: [row({ tweaks: shaped })] });
    const { rerender } = render(
      <PadEditor entry={row({ tweaks: shaped })} onClose={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Store B/i }));

    const changed = { ...shaped, adsr: { ...shaped.adsr, attackMs: 20 } };
    useKit.setState({ lanes: [row({ tweaks: changed })] });
    rerender(<PadEditor entry={row({ tweaks: changed })} onClose={() => {}} />);

    fireEvent.click(screen.getByRole('button', { name: /Keep B/i }));
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('pad_tweaks_set', {
        lane: 'kick',
        tweaks: { ...changed, adsr: shaped.adsr },
      }),
    );
  });
});
