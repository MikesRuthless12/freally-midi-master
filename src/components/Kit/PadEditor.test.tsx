import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

const { PadEditor, envelopePath } = await import('./PadEditor');
const { useKit } = await import('../../state/kit');
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

function plainRow() {
  return {
    lane: 'kick' as const,
    shipped: true,
    name: null as string | null,
    path: null as string | null,
    tweaks: untouchedPad(),
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
    const sent = invoke.mock.calls[0]?.[1] as { tweaks: { trimStart: number; trimEnd: number } };
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
