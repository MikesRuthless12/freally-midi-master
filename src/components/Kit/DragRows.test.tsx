import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { DragRows } from './DragRows';
import { useDrag } from '../../state/drag';
import { useKit } from '../../state/kit';
import { useSession } from '../../state/session';
import { useSong } from '../../state/song';
import type { Pattern } from '../../lib/ipc-types';

/**
 * Reaching one instrument out of a part, and all of them at once (2026-08-06).
 *
 * ⛔ **Mike reported this as missing while half of it was already built.** The
 * per-lane split existed behind a "Per lane" toggle in the Stems panel — a
 * control that reads like an export preference — and he was looking straight at
 * the panel when he said the drums could only be dragged as one lump. So these
 * tests are about *reachability*, not capability: what has to hold is that the
 * instruments are one press from the row, and that "All Tracks" exists at all,
 * which it genuinely did not.
 *
 * ⚠ Driven through the component rather than the store, because the store was
 * never the thing that was wrong.
 */

const DRUMS: Pattern = {
  id: 'trap-drums',
  part: 'drums',
  artistId: 'trap',
  seed: '1',
  bars: 4,
  bpm: 140,
  timeSigNum: 4,
  timeSigDen: 4,
  keyRoot: 6,
  scale: 'natural_minor',
  ppq: 960,
  lanes: [
    { lane: 'kick', notes: [{ startTick: 0, lenTicks: 240, pitch: 36, vel: 110 }] },
    { lane: 'snare', notes: [{ startTick: 960, lenTicks: 240, pitch: 38, vel: 100 }] },
    { lane: 'closedHat', notes: [{ startTick: 0, lenTicks: 120, pitch: 42, vel: 80 }] },
    // ⛔ Present and silent. A menu that offered this would hand back a file the
    // producer imports and hears nothing from — the "silent files called stems"
    // failure `audio/render.rs` exists to record.
    { lane: 'openHat', notes: [] },
  ],
};

const BASS: Pattern = {
  ...DRUMS,
  id: 'trap-bass',
  part: 'bass',
  lanes: [{ lane: 'bass', notes: [{ startTick: 0, lenTicks: 480, pitch: 28, vel: 100 }] }],
};

/**
 * The kit as the plugin reports it.
 *
 * ⛔ `snap` is shipped-silent on purpose and it is not hypothetical: the drum
 * generator writes that lane and no shipped pad has ever played it.
 */
const KIT = [
  { lane: 'kick' as const, shipped: true, name: null, path: null },
  { lane: 'snare' as const, shipped: true, name: null, path: null },
  { lane: 'closedHat' as const, shipped: true, name: null, path: null },
  { lane: 'openHat' as const, shipped: true, name: null, path: null },
  { lane: 'snap' as const, shipped: false, name: null, path: null },
  { lane: 'bass' as const, shipped: true, name: null, path: null },
];

beforeEach(() => {
  // The rows render only where the plugin says a native drag source exists.
  useDrag.setState({ canDrag: true, state: 'idle', message: null });
  useSession.setState({ patterns: { drums: DRUMS } });
  useSong.setState({ song: null });
  useKit.setState({ lanes: KIT, loaded: true });
});

afterEach(cleanup);

/** The menu hanging off a format button in the row named `part`. */
function openMenu(rowLabel: string, format: string) {
  const row = screen.getByText(rowLabel).closest('li') as HTMLElement;
  fireEvent.click(within(row).getByRole('button', { name: format }));
  return row;
}

describe('one part, one instrument at a time', () => {
  it('lists every instrument that is playing, and none that are not', () => {
    render(<DragRows />);
    const row = openMenu('Drums', 'MIDI');

    expect(within(row).getByRole('button', { name: 'Kick' })).toBeTruthy();
    expect(within(row).getByRole('button', { name: 'Snare' })).toBeTruthy();
    expect(within(row).getByRole('button', { name: 'Closed hat' })).toBeTruthy();
    // Authored but empty — offering it would spool a file of nothing.
    expect(within(row).queryByRole('button', { name: 'Open hat' })).toBeNull();
  });

  it('offers All Tracks last, which is the part nothing could do before', () => {
    // ⛔ Export wrote every lane in one action and drag could not, so eight
    // instruments meant eight separate gestures across into the DAW.
    render(<DragRows />);
    const row = openMenu('Drums', 'MIDI');

    // ⚠ The menu's own entries, not every button in the row — the row also
    // holds the two format openers, and the Audio one sits after the MIDI menu
    // in the DOM.
    // ⚠ **In `LANE_ORDER`, which is the drum grid's own top-to-bottom order**
    // (hats above snare above kick), not the order the engine emitted them in.
    // A producer reading the menu is looking for the row they can see.
    //
    // ⛔⛔ **"As one clip" is FIRST, and it is the gesture the menu took away.**
    // Before this menu existed the format button was itself a draggable chip
    // whose subject named no lane — `Cut::Parts`, one file holding the whole
    // kit, onto one DAW track. Making it a menu opener turned it into a plain
    // `<button onClick>` that cannot be dragged, so that had no route left:
    // one instrument at a time, or All Tracks, which is eight separate files.
    const entries = within(row)
      .getAllByRole('listitem')
      .map((item) => item.textContent);
    expect(entries).toEqual(['As one clip', 'Closed hat', 'Snare', 'Kick', 'All Tracks']);
  });

  it('can still drag a whole multi-lane part out as a single file', () => {
    // ⛔ The assertion above pins the *order*; this pins that the entry is a
    // real drag source rather than a label — a chip, addressable, and naming no
    // lane, which is what makes the plugin cut it as one file.
    render(<DragRows />);
    const row = openMenu('Drums', 'MIDI');

    const whole = within(row).getByRole('button', { name: 'As one clip' });
    expect(whole).toBeTruthy();
    // A `<button>` inside the list, not the menu opener that sits outside it.
    expect(whole.closest('.drag__lane')).toBeTruthy();
  });

  it('still offers Audio when the KIT panel has never been opened', async () => {
    // ⛔⛔ **The whole Audio drag-out used to disappear.** `useKit.refresh()` is
    // the only caller of `kit_state`, and it ran exclusively from `KitPanel`'s
    // mount effect — but `layout/Section.tsx` renders children behind
    // `{open && …}` and the KIT section's open state is persisted. So a
    // producer who collapsed KIT once and reloaded had `lanes === []` for the
    // rest of the session: every Audio chip vanished, on every row, with
    // nothing on screen explaining why, while Export still offered audio.
    //
    // ⚠ This is the state that reproduces it: never asked, not answered-empty.
    useKit.setState({ lanes: [], loaded: false });
    render(<DragRows />);

    const row = screen.getByText('Drums').closest('li') as HTMLElement;
    expect(within(row).getByRole('button', { name: 'Audio' })).toBeTruthy();

    // And the panel loads the kit itself rather than waiting for another one.
    await waitFor(() => expect(useKit.getState().loaded).toBe(true));
  });

  it('gives Audio the same menu as MIDI', () => {
    // Mike: "the same goes for the audio tracks".
    render(<DragRows />);
    const row = openMenu('Drums', 'Audio');

    expect(within(row).getByRole('button', { name: 'Kick' })).toBeTruthy();
    expect(within(row).getByRole('button', { name: 'All Tracks' })).toBeTruthy();
  });

  it('says whether it is open, so the button is not a mystery', () => {
    render(<DragRows />);
    const row = screen.getByText('Drums').closest('li') as HTMLElement;
    const opener = within(row).getByRole('button', { name: 'MIDI' });

    expect(opener.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(opener);
    expect(opener.getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(opener);
    expect(opener.getAttribute('aria-expanded')).toBe('false');
  });

  it('closes on Escape rather than trapping the producer in it', () => {
    render(<DragRows />);
    const row = openMenu('Drums', 'MIDI');
    expect(within(row).queryByRole('button', { name: 'Kick' })).toBeTruthy();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(within(row).queryByRole('button', { name: 'Kick' })).toBeNull();
  });

  /**
   * MIDI follows what was written; audio follows what can be played.
   *
   * ⛔⛔ **Mike, 2026-08-06:** *"you should be able to drag the midi from a lane
   * that's been authored but has no sound, but not the audio unless it has a
   * sample associated with it."* The two chips answer different questions and
   * the first cut of this menu asked only one of them.
   */
  describe('a lane the kit cannot play', () => {
    const WITH_SNAP: Pattern = {
      ...DRUMS,
      lanes: [
        ...DRUMS.lanes,
        { lane: 'snap', notes: [{ startTick: 480, lenTicks: 120, pitch: 39, vel: 90 }] },
      ],
    };

    it('is offered as MIDI, because the notes were written either way', () => {
      // A producer routing this into Battery wants the lane our kit has no pad
      // for — the notes are real, and only *our* preview is silent on them.
      useSession.setState({ patterns: { drums: WITH_SNAP } });
      render(<DragRows />);
      const row = openMenu('Drums', 'MIDI');

      expect(within(row).getByRole('button', { name: 'Snap' })).toBeTruthy();
    });

    it('is not offered as audio, because there is nothing to render', () => {
      useSession.setState({ patterns: { drums: WITH_SNAP } });
      render(<DragRows />);
      const row = openMenu('Drums', 'Audio');

      expect(within(row).queryByRole('button', { name: 'Snap' })).toBeNull();
      // The lanes that can sound are still all there.
      expect(within(row).getByRole('button', { name: 'Kick' })).toBeTruthy();
    });

    it('is offered as audio once the producer assigns their own sample', () => {
      // ⚠ A one-shot is a sample too. `audio/render.rs` writes a file for
      // either, so the menu must not insist on the *shipped* voice.
      useSession.setState({ patterns: { drums: WITH_SNAP } });
      useKit.setState({
        lanes: KIT.map((one) =>
          one.lane === 'snap' ? { ...one, name: 'my-snap.wav', path: 'C:/s/my-snap.wav' } : one,
        ),
      });
      render(<DragRows />);
      const row = openMenu('Drums', 'Audio');

      expect(within(row).getByRole('button', { name: 'Snap' })).toBeTruthy();
    });

    it('takes the Audio chip away entirely when nothing in the part can sound', () => {
      // ⚠ Not an empty menu — a handle that opens onto nothing is the
      // readout-that-lies failure in miniature.
      useSession.setState({
        patterns: {
          drums: {
            ...DRUMS,
            lanes: [
              { lane: 'snap', notes: [{ startTick: 0, lenTicks: 120, pitch: 39, vel: 90 }] },
            ],
          },
        },
      });
      render(<DragRows />);
      const row = screen.getByText('Drums').closest('li') as HTMLElement;

      expect(within(row).getByRole('button', { name: 'MIDI' })).toBeTruthy();
      expect(within(row).queryByRole('button', { name: 'Audio' })).toBeNull();
    });
  });

  /**
   * The four melodic generators, in one gesture (Mike, 2026-08-06).
   *
   * *"you should also be able to drag all 4 of the other generators midi/audio
   * all at the same time too into the DAW (melody/bassline/countermelody/
   * chords), just the drums has to be separate because it has it's own separate
   * lanes per the generator."*
   */
  describe('all the melodic parts at once', () => {
    const MELODY: Pattern = {
      ...BASS,
      id: 'trap-melody',
      part: 'melody',
      lanes: [{ lane: 'melody', notes: [{ startTick: 0, lenTicks: 240, pitch: 72, vel: 90 }] }],
    };

    it('offers one handle for every generated melodic part', () => {
      useSession.setState({ patterns: { drums: DRUMS, bass: BASS, melody: MELODY } });
      render(<DragRows />);

      expect(screen.getByText('All Parts')).toBeTruthy();
    });

    it('leaves the drums out of it, because its instruments are its lanes', () => {
      // ⛔ Drums has its own All Tracks inside its menu, and it means something
      // different — every *lane*, not every *part*. Folding it in would make
      // one gesture mean two things depending on which part it caught.
      useSession.setState({ patterns: { drums: DRUMS, bass: BASS, melody: MELODY } });
      render(<DragRows />);
      const row = screen.getByText('All Parts').closest('li') as HTMLElement;

      // The row drags exactly the melodic parts; the drum row is still its own.
      expect(within(row).getByRole('button', { name: 'MIDI' })).toBeTruthy();
      expect(screen.getByText('Drums')).toBeTruthy();
    });

    it('is not offered when only one part could go in it', () => {
      // ⚠ It would be a second way to drag the row directly above it.
      useSession.setState({ patterns: { drums: DRUMS, bass: BASS } });
      render(<DragRows />);

      expect(screen.queryByText('All Parts')).toBeNull();
    });
  });

  it('gives a single-instrument part a plain chip instead of a menu of one', () => {
    // ⚠ A bassline *is* its instrument. A menu here would be a press that opens
    // a list repeating the row's own name back at the producer.
    useSession.setState({ patterns: { bass: BASS } });
    render(<DragRows />);
    const row = screen.getByText('Bass').closest('li') as HTMLElement;

    expect(
      within(row).getByRole('button', { name: 'MIDI' }).getAttribute('aria-expanded'),
    ).toBe(null);
  });
});
