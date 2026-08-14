import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { DragRows } from './DragRows';
// ⚠ The mock's own idea of an unedited pad, rather than a second one written
// here — see its doc for why a fixture with two spellings is the drift this
// codebase keeps recording.
import { untouchedPad } from '../../lib/ipc-mock';
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
  songSeed: '1',
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
  { lane: 'kick' as const, shipped: true, name: null, path: null, tweaks: untouchedPad() },
  { lane: 'snare' as const, shipped: true, name: null, path: null, tweaks: untouchedPad() },
  { lane: 'closedHat' as const, shipped: true, name: null, path: null, tweaks: untouchedPad() },
  { lane: 'openHat' as const, shipped: true, name: null, path: null, tweaks: untouchedPad() },
  { lane: 'snap' as const, shipped: false, name: null, path: null, tweaks: untouchedPad() },
  { lane: 'bass' as const, shipped: true, name: null, path: null, tweaks: untouchedPad() },
];

beforeEach(() => {
  // The rows render only where the plugin says a native drag source exists.
  useDrag.setState({ canDrag: true, state: 'idle', message: null });
  useSession.setState({ patterns: { drums: DRUMS } });
  useSong.setState({ song: null });
  useKit.setState({ lanes: KIT, loaded: true });
});

afterEach(cleanup);

/**
 * Open the menu hanging off a format button in the row named `part`, and answer
 * **the menu** rather than the row.
 *
 * ⛔ **The menu is rendered into `document.body`, so it is NOT a descendant of
 * the row.** `.drag__lanes` used to be `position: absolute` inside
 * `.rail__content`, which is `overflow-y: auto` — so the scroll container cut it
 * off partway down and the lower drum lanes could not be reached at all. Mike
 * screenshotted that on 2026-08-06, with Snare the last row he could see.
 * Escaping the clip means escaping the row in the DOM, so **menu entries are
 * queried through this and the format openers still through the row.**
 */
function openMenu(rowLabel: string, format: string) {
  const row = screen.getByText(rowLabel).closest('li') as HTMLElement;
  fireEvent.click(within(row).getByRole('button', { name: format }));
  return document.querySelector('.drag__lanes') as HTMLElement;
}

describe('one part, one instrument at a time', () => {
  // ⛔⛔ **BOTH formats, and the audio half is why this test exists.** Mike,
  // 2026-08-06, after the MIDI menu was un-clipped: *"ensure that the menu is
  // sitting above and doesn't get caught behind the other controls as well like
  // the other one did early this morning for the midi drum lanes menu."*
  //
  // The clipping was never about z-order — `.drag__lanes` was `position:
  // absolute` inside `.rail__content`, which is `overflow-y: auto`, and a
  // scroll container crops a descendant whatever its z-index is. So the
  // property that actually fixes it is **escaping the subtree**, and that is
  // what is asserted here. ⚠ The helper above documents the portal but nothing
  // pinned it, so re-parenting the menu back into the row would have gone
  // unnoticed until a producer screenshotted it a second time.
  it.each(['MIDI', 'Audio'])('opens the %s menu outside the scroll container', (format) => {
    render(<DragRows />);
    const row = screen.getByText('Drums').closest('li') as HTMLElement;
    const menu = openMenu('Drums', format);

    expect(menu).toBeTruthy();
    expect(menu.parentElement).toBe(document.body);
    expect(row.contains(menu)).toBe(false);
  });

  it('lists every instrument that is playing, and none that are not', () => {
    render(<DragRows />);
    const menu = openMenu('Drums', 'MIDI');

    expect(within(menu).getByRole('button', { name: 'Kick' })).toBeTruthy();
    expect(within(menu).getByRole('button', { name: 'Snare' })).toBeTruthy();
    expect(within(menu).getByRole('button', { name: 'Closed hat' })).toBeTruthy();
    // Authored but empty — offering it would spool a file of nothing.
    expect(within(menu).queryByRole('button', { name: 'Open hat' })).toBeNull();
  });

  it('gives the MIDI menu the instruments and nothing else', () => {
    // ⛔ Export wrote every lane in one action and drag could not, so eight
    // instruments meant eight separate gestures across into the DAW. All Tracks
    // answered that — for audio. It is absent here by instruction.
    render(<DragRows />);
    const menu = openMenu('Drums', 'MIDI');

    // ⚠ The menu's own entries, not every button in the row — the row also
    // holds the two format openers, and the Audio one sits after the MIDI menu
    // in the DOM.
    // ⚠ **In `LANE_ORDER`, which is the drum grid's own top-to-bottom order**
    // (hats above snare above kick), not the order the engine emitted them in.
    // A producer reading the menu is looking for the row they can see.
    //
    // ⛔⛔ **NEITHER "As one clip" NOR "All Tracks" IS HERE, AND THAT IS THE
    // POINT.** Mike, 2026-08-06: *"you shouldn't be able to drag the midi
    // altogether for the drums, only the audio … there should never be one
    // single midi file with all parts of the drums draggable"*, and then: *"the
    // 'MIDI' should not have an 'All Tracks', only the audio should."* One
    // `.mid` of the whole kit is eight instruments stacked on one track. The
    // audio menu keeps both entries, and the test below pins that half.
    const entries = within(menu)
      .getAllByRole('listitem')
      .map((item) => item.textContent);
    expect(entries).toEqual(['Closed hat', 'Snare', 'Kick']);
  });

  it('never offers the whole drum kit as one MIDI file', () => {
    // ⛔⛔ Mike's rule, pinned as a *refusal* rather than as an ordering, so it
    // survives someone reinstating the entry without reading the line above.
    // Mike again, 2026-08-06: *"i just don't want an altogether midi file."*
    render(<DragRows />);
    const menu = openMenu('Drums', 'MIDI');

    expect(within(menu).queryByRole('button', { name: 'All Tracks' })).toBeNull();
    expect(within(menu).queryByRole('button', { name: 'As one clip' })).toBeNull();
  });

  it('offers the audio lanes separately, like MIDI, plus All Tracks as one file', () => {
    // ⛔⛔ **INVERTED 2026-08-06, and the inversion is the fix.** This used to
    // read `['As one clip', 'Closed hat', 'Snare', 'Kick', 'All Tracks']`, where
    // "All Tracks" meant one SEPARATE file per lane and "As one clip" was the
    // mix. Mike read the label the other way round — pressed the mix, got one
    // track, and reported it: *"not just one track altogether and that's it."*
    //
    // ▶ His instruction: *"i want the separate audio drum lanes just like the
    // midi lanes, and then an 'All Tracks' for all the tracks mixed for the
    // audio lanes"* … *"i want all the clips in one file for 'All Tracks'."*
    // So the separate half is the per-lane chips — identical to the MIDI menu —
    // and "All Tracks" is now the single mixed file. "As one clip" is gone
    // because "All Tracks" *is* it.
    render(<DragRows />);
    const menu = openMenu('Drums', 'Audio');

    // ⚠ Asserted as menu *contents* rather than by inspecting the opener's
    // classes: `DragChip` and the plain MIDI button render identical
    // `className`s, so a class check would pass either way and prove nothing.
    const entries = within(menu)
      .getAllByRole('listitem')
      .map((item) => item.textContent);
    // ⚠ The lanes ahead of "All Tracks" are the same three the MIDI menu
    // offers — *"just like the midi lanes"* — and that half is pinned by
    // `gives the MIDI menu the instruments and nothing else` above. Asserting it
    // again here would mean opening both menus at once, and both portal into
    // `document.body`, so `within` picks up whichever is still mounted.
    expect(entries).toEqual(['Closed hat', 'Snare', 'Kick', 'All Tracks']);

    // A `<button>` inside the list, not the menu opener that sits outside it —
    // and "outside" is now a different subtree entirely, since the list is
    // portalled out of the scroll container that used to crop it.
    const whole = within(menu).getByRole('button', { name: 'All Tracks' });
    expect(whole.closest('.drag__lane')).toBeTruthy();
    const row = screen.getByText('Drums').closest('li') as HTMLElement;
    expect(
      within(row).getByRole('button', { name: 'Audio' }).closest('.drag__lane'),
    ).toBeNull();
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
    const menu = openMenu('Drums', 'Audio');

    expect(within(menu).getByRole('button', { name: 'Kick' })).toBeTruthy();
    expect(within(menu).getByRole('button', { name: 'All Tracks' })).toBeTruthy();
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
    const menu = openMenu('Drums', 'MIDI');
    expect(within(menu).queryByRole('button', { name: 'Kick' })).toBeTruthy();

    fireEvent.keyDown(document, { key: 'Escape' });
    // ⚠ Re-queried rather than reusing `menu`: closing unmounts the portal, so
    // the assertion is that the list is gone from the document altogether.
    expect(document.querySelector('.drag__lanes')).toBeNull();
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
      const menu = openMenu('Drums', 'MIDI');

      expect(within(menu).getByRole('button', { name: 'Snap' })).toBeTruthy();
    });

    it('is offered as audio too, because every lane offers both formats', () => {
      // ⛔⛔ **THIS ASSERTION IS THE INVERSE OF THE ONE IT REPLACES**, which read
      // `queryByRole(...'Snap').toBeNull()` under the title *"is not offered as
      // audio, because there is nothing to render"*. Mike, 2026-08-06: *"each
      // individual drum lane should be able to drag midi or audio, not just
      // midi and not just audio."*
      //
      // ⚠ The old rule was not wrong about the consequence — a lane the kit has
      // no pad for spools a **silent** file — it was wrong about the remedy.
      // Offering Kick as both and Snap as MIDI-only reads as a broken menu.
      useSession.setState({ patterns: { drums: WITH_SNAP } });
      render(<DragRows />);
      const menu = openMenu('Drums', 'Audio');

      expect(within(menu).getByRole('button', { name: 'Snap' })).toBeTruthy();
      expect(within(menu).getByRole('button', { name: 'Kick' })).toBeTruthy();
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
      const menu = openMenu('Drums', 'Audio');

      expect(within(menu).getByRole('button', { name: 'Snap' })).toBeTruthy();
    });

    it('still offers Audio for a part only the producer can make sound', () => {
      // ⛔⛔ **ALSO INVERTED, and by the same instruction.** This read
      // `queryByRole(...'Audio').toBeNull()` under *"takes the Audio chip away
      // entirely when nothing in the part can sound"*, whose reasoning was that
      // a handle opening onto nothing is the readout-that-lies failure in
      // miniature. That still holds for an *empty* menu — and this menu is not
      // empty, because every written lane is now in it.
      //
      // ⚠ What the producer gets from a lane the shipped kit has no pad for is
      // a silent render, until they assign their own sample. Mike chose that
      // over a chip that disappears with no explanation.
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
      expect(within(row).getByRole('button', { name: 'Audio' })).toBeTruthy();
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

    it('takes the drums in with everything else', () => {
      // ⛔⛔ **INVERTED 2026-08-11.** This read "leaves the drums out of it,
      // because its instruments are its lanes" and came from Mike's own
      // 2026-08-06 sentence — *"just the drums has to be separate because it has
      // it's own separate lanes per the generator."* He has since named drums in
      // the list: *"it should be all parts of the chords/melody/counter melody/
      // basslines/drums one clip after the next."*
      //
      // ⚠ The rule that did **not** move is the one about the Drums row's own
      // MIDI chip — no whole-kit `.mid` from there — and the test above still
      // pins it. The drums arriving as one clip *inside a five-clip sequence* is
      // a different gesture.
      useSession.setState({ patterns: { drums: DRUMS, bass: BASS, melody: MELODY } });
      render(<DragRows />);
      const row = screen.getByText('All Parts').closest('li') as HTMLElement;

      expect(within(row).getByRole('button', { name: 'MIDI' })).toBeTruthy();
      // And the drum row is still its own, with its own per-lane menu.
      expect(screen.getByText('Drums')).toBeTruthy();
    });

    it('is not offered when only one part could go in it', () => {
      // ⚠ It would be a second way to drag the row directly above it.
      useSession.setState({ patterns: { drums: DRUMS } });
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

  /**
   * ⛔⛔ **THE MELODIC PARTS HAD NO AUDIO CHIP AT ALL, ON ANY KIT** (2026-08-11).
   *
   * Mike: *"when I have samples dragged to my kit for the Melody/Chords/Counter
   * melody, they do not have an 'Audio' button able to drag audio to my DAW, but
   * they play in the generators."* The samples were a red herring — `written`
   * was built as `LANE_ORDER.filter(…)`, and `LANE_ORDER` is the **drum** lane
   * list, so it matched nothing for a part whose lane is `melody`, `chords`,
   * `counter` or `bass`. `Row.audio` is `written.length > 0`, so the chip could
   * never appear whatever was on the pads.
   *
   * ⚠ **Every one of the four is named**, rather than one standing for the rest:
   * the bug was that a whole *class* of lane fell outside a list, and a single
   * example would go on passing if three of the four were dropped again.
   */
  describe('a melodic part drags as audio too', () => {
    const melodic = (part: string, lane: string): Pattern => ({
      ...BASS,
      id: `trap-${part}`,
      part: part as Pattern['part'],
      lanes: [{ lane: lane as Pattern['lanes'][number]['lane'], notes: BASS.lanes[0].notes }],
    });

    it.each([
      ['Melody', 'melody', 'melody'],
      ['Chords', 'chords', 'chords'],
      ['Counter', 'counter', 'counter'],
      ['Bass', 'bass', 'bass'],
    ])('offers %s an Audio chip beside its MIDI one', (label, part, lane) => {
      useSession.setState({ patterns: { [part]: melodic(part, lane) } });
      render(<DragRows />);
      const row = screen.getByText(label).closest('li') as HTMLElement;

      expect(within(row).getByRole('button', { name: 'MIDI' })).toBeTruthy();
      expect(within(row).getByRole('button', { name: 'Audio' })).toBeTruthy();
    });

    it('withholds Audio from a part the generator never wrote to', () => {
      // ⚠ The half of the old rule that was right, and it must survive the fix:
      // widening `written` past the drum lanes must not start offering audio for
      // a lane that carries no notes, because that render is a file of nothing.
      useSession.setState({
        patterns: {
          melody: { ...BASS, part: 'melody', lanes: [{ lane: 'melody', notes: [] }] },
        },
      });
      render(<DragRows />);
      const row = screen.getByText('Melody').closest('li') as HTMLElement;

      expect(within(row).queryByRole('button', { name: 'Audio' })).toBeNull();
    });

    it('still reads a drum menu in the grid order rather than the engine order', () => {
      // ⚠ `LANE_ORDER` is now a *ranking* rather than a filter, so this is the
      // property that could have been lost silently: hats above snare above
      // kick is how the drum grid draws them, and the menu has to agree.
      render(<DragRows />);
      const menu = openMenu('Drums', 'MIDI');

      const entries = within(menu)
        .getAllByRole('listitem')
        .map((item) => item.textContent);
      expect(entries).toEqual(['Closed hat', 'Snare', 'Kick']);
    });
  });
});
