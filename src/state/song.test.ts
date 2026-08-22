import { beforeEach, expect, it, vi } from 'vitest';

import type { Part, Song } from '../lib/ipc-types';

/**
 * The arrangement document (TASK-067).
 *
 * ⛔ **The half these tests exist for is persistence, and it is invisible.**
 * Every gesture in the timeline already has a spec that asserts the resulting
 * geometry; none of them can see whether the result would survive closing the
 * project, which is the failure three consecutive handoffs have written down.
 * So what is asserted here is the *payload* — that an edited song is in the one
 * value the host saves, and that an unedited one is deliberately not.
 */

const invoke = vi.fn();
vi.mock('../lib/ipc', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

// The plugin gate: `persist()` is a no-op in a browser, so without this the
// payload assertions below would pass by never being sent at all.
vi.mock('../lib/ipc-plugin', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../lib/ipc-plugin')>()),
  isPlugin: () => true,
}));

const { useSong } = await import('./song');
const { useSession } = await import('./session');
const { useHistory, canRedo, canUndo } = await import('./history');
const { useUi } = await import('./ui');

/** Two sections, the second sharing nothing with the first. */
function song(): Song {
  const clip = (id: string) => ({
    id,
    part: 'drums' as Part,
    artistId: 'trap',
    seed: '7',
    songSeed: '7',
    bars: 4,
    bpm: 140,
    timeSigNum: 4,
    timeSigDen: 4,
    keyRoot: 6,
    scale: 'natural_minor' as const,
    lanes: [],
    ppq: 960,
    mood: null,
    base: null,
    loopRegion: null,
    clipRegion: null,
  });
  return {
    id: 'trap-song-7',
    artistId: 'trap',
    seed: '7',
    bpm: 140,
    keyRoot: 6,
    scale: 'natural_minor',
    sections: [
      {
        type: 'intro',
        startBar: 0,
        bars: 4,
        patterns: { drums: { patternId: 'a' } } as Song['sections'][number]['patterns'],
        dropOutBeats: 0,
        decay: false,
        markers: [],
      },
      {
        type: 'hook',
        startBar: 4,
        bars: 8,
        patterns: { drums: { patternId: 'b' } } as Song['sections'][number]['patterns'],
        dropOutBeats: 0,
        decay: false,
        markers: [],
      },
    ],
    timeSigNum: 4,
    timeSigDen: 4,
    patterns: { a: clip('a'), b: clip('b') },
    ppq: 960,
  };
}

/** The last `save_session_state` payload, or null if none was sent. */
function lastSave(): Record<string, unknown> | null {
  const calls = invoke.mock.calls.filter(([command]) => command === 'save_session_state');
  const last = calls.at(-1);
  return last ? ((last[1] as { session: Record<string, unknown> }).session ?? null) : null;
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  vi.useFakeTimers();
  useSong.setState({
    song: null,
    edited: false,
    generating: false,
    error: null,
    selection: [],
    anchor: null,
    clipboard: null,
    locks: [],
    // ⛔ Reset with the rest. Left out, the solo test's `soloParts` survived into
    // the mute test below it and made a *correct* implementation look wrong —
    // and would equally have let a broken one pass, which is the half that
    // matters.
    loopSection: null,
    mutedParts: [],
    soloParts: [],
    audition: null,
    drillPatternId: null,
    drillSongId: null,
    structure: null,
  });
  useSession.setState({ patterns: {}, edited: false });
  // ⛔ `armSong` gates on the visible tab — there is one schedule and the tab
  // decides whose it is. Every case here is about Song Mode, so this is the
  // tab they run on; the one case that asserts the *gate* sets its own.
  useUi.setState({ activeTab: 'song' });
});

/** Run the 300 ms save debounce out. */
function flushSave() {
  vi.runAllTimers();
}

/**
 * Put an arrangement on screen with a history baseline behind it.
 *
 * ⛔ Armed, not merely present. `history.record` is a no-op until `arm` has
 * established the point undo cannot go behind, so a test that skipped this
 * would record nothing and every undo assertion below would pass by never
 * having anything to undo.
 */
function armedWith(current = song()) {
  // ⛔ **The artist first, then the song, and the order is not cosmetic.**
  // `snapshotOf` compares `song.artistId` to `selectedId` — the guard that stops
  // a snapshot carrying the previous artist's record — so without a matching
  // artist every snapshot recorded here would silently hold `song: null` and
  // the undo assertions would be testing nothing. And setting it *after* the
  // song trips the artist-change subscriber, which clears the song outright.
  useSession.setState({ selectedId: current.artistId });
  useSong.setState({ song: current });
  useHistory.getState().arm({
    selectedId: 'trap',
    seed: '7',
    songSeed: '7',
    seedPinned: true,
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
    complexity: 'authored' as const,
    patterns: {},
    editedParts: [],
    edited: false,
    mood: null,
    base: null,
    audioEnabled: true,
    mutedLanes: [],
    soloedLanes: [],
    lockedLanes: [],
    partsOff: [],
    song: current,
    songEdited: false,
    // The kit the snapshot was taken over (TASK-050A). Empty: these fixtures
    // are about the session fields, and a kit belongs to the plugin.
    oneShots: {},
  });
  return current;
}

it('an edited arrangement is in the value the host saves', () => {
  useSong.setState({ song: song() });
  useSong.getState().resize(0, 8);
  flushSave();

  const saved = lastSave();
  expect(saved).not.toBeNull();
  expect(saved?.songEdited).toBe(true);
  expect((saved?.song as Song).sections[0].bars).toBe(8);
});

it('an unedited arrangement is deliberately not saved', () => {
  // ⛔ The property that keeps a project file small: a song still describable by
  // its seed is regenerated by pressing Generate, so storing kilobytes of notes
  // to restore it would buy nothing — and would stop the project picking up
  // engine improvements, which `state.rs` documents as the whole trade.
  useSong.setState({ song: song(), edited: false });
  useSession.getState().setSeed('12');
  flushSave();

  const saved = lastSave();
  expect(saved).not.toBeNull();
  expect(saved).not.toHaveProperty('song');
  expect(saved?.songEdited).toBeUndefined();
});

it('a resize that changes nothing is not an edit and is not saved as one', () => {
  // `clips.ts` returns the same object on a no-op, which is what makes `edited`
  // honest. Marking it would tell the producer their song no longer matches its
  // seed when it does — and would put the whole arrangement in the project file
  // for a gesture that did nothing.
  useSong.setState({ song: song() });
  useSong.getState().resize(0, 4);
  flushSave();

  expect(useSong.getState().edited).toBe(false);
  expect(lastSave()).toBeNull();
});

it('resizing a clip loops that one row without touching its section', () => {
  // ⛔⛔ **TASK-142's clip resize, and the two gestures must stay apart.** Mike's
  // review: *"there is no clip resize"* — the timeline could only change a
  // *section's* length, which moves every row at once. This changes how many
  // bars one row loops on, and the section it sits in is untouched.
  useSong.setState({ song: song() });
  useSong.getState().resizeClip({ sectionIndex: 0, part: 'drums' }, 2);

  const after = useSong.getState().song!;
  expect(after.sections[0].patterns.drums?.bars).toBe(2);
  expect(after.sections[0].bars).toBe(song().sections[0].bars);
  // ...and only that clip: the second section plays the same pattern id, and
  // writing the length on the *pattern* rather than the reference would have
  // shortened it too. That is the whole reason the field lives where it does.
  expect(after.sections[1].patterns.drums?.bars ?? null).toBeNull();
});

it('a clip dragged back out to full length stops claiming a length at all', () => {
  // ⚠ `null`, not the number. A song resized and un-resized has to come out
  // byte-identical to one that was never touched — otherwise every project
  // carries a field saying "four bars" that a later edit to the pattern would
  // then silently contradict.
  useSong.setState({ song: song() });
  useSong.getState().resizeClip({ sectionIndex: 0, part: 'drums' }, 2);
  useSong.getState().resizeClip({ sectionIndex: 0, part: 'drums' }, 4);

  expect(useSong.getState().song!.sections[0].patterns.drums?.bars ?? null).toBeNull();
});

it('a clip cannot be resized to nothing, or past its own notes', () => {
  // ⛔ Zero would ask the engine to lay the clip down once per tick — its own
  // guard refuses that, and a UI that can send it is a UI relying on a guard.
  // Longer than the pattern is not a longer loop: there are no notes out there,
  // so it would read as the clip having gone quiet.
  useSong.setState({ song: song() });
  useSong.getState().resizeClip({ sectionIndex: 0, part: 'drums' }, 0);
  expect(useSong.getState().song!.sections[0].patterns.drums?.bars).toBe(1);

  useSong.getState().resizeClip({ sectionIndex: 0, part: 'drums' }, 99);
  expect(useSong.getState().song!.sections[0].patterns.drums?.bars ?? null).toBeNull();
});

it('a re-roll resolves the producer’s locks into the parts the engine wants', async () => {
  // The engine is deliberately lock-agnostic — it takes a list of parts to
  // leave alone — so everything about *how* a lock was expressed is resolved on
  // this side. A lock on another section must not narrow this one's re-roll.
  const current = song();
  invoke.mockResolvedValue(current);
  useSong.setState({ song: current, locks: ['0:drums', '1:melody'] });

  await useSong.getState().reroll(0, null);

  const call = invoke.mock.calls.find(([command]) => command === 'reroll_section');
  expect(call).toBeDefined();
  expect((call?.[1] as { request: { locked: Part[] } }).request.locked).toEqual(['drums']);
});

it('both song doors carry the Simple/Complex switch the producer set', async () => {
  // ⛔⛔ **The `base` failure of TASK-158C, one field over, and it shipped.**
  // `bridge.rs` reads `complexity` on `generate_song` and on `reroll_section`,
  // and both reads were written correctly — the page simply never sent the
  // field, so Song Mode arranged at the model's own reading while every
  // four-bar loop on the part tabs beside it answered Busy, and re-rolling one
  // section brought it back plainer than its neighbours with nothing on screen
  // saying why. A door nobody knocks on is the same defect as a door that was
  // never built, so this asserts on the *payload* rather than on the store.
  const current = song();
  invoke.mockResolvedValue(current);
  useSession.setState({ complexity: 'complex' });
  useSong.setState({ song: current });

  await useSong.getState().generate({
    styleId: 'trap',
    seed: '9',
    pins: {} as never,
    mood: null,
    base: null,
    complexity: 'complex',
  });
  await useSong.getState().reroll(0, null);

  for (const command of ['generate_song', 'reroll_section'] as const) {
    const call = invoke.mock.calls.find(([name]) => name === command);
    expect(call, `${command} should have been invoked`).toBeDefined();
    expect(
      (call?.[1] as { request: { complexity?: string } }).request.complexity,
      `${command} arranged at a different reading from the loops beside it`,
    ).toBe('complex');
  }
});

it('a re-roll marks the arrangement edited, because its seed no longer describes it', async () => {
  // ⛔ The edit that is easiest to lose: nothing about a re-roll *looks* like an
  // edit. The timeline redraws and the geometry is identical, so without this
  // the section would silently revert to the seed's own notes on reopen.
  const current = song();
  invoke.mockResolvedValue(current);
  useSong.setState({ song: current });

  await useSong.getState().reroll(1, null);
  flushSave();

  expect(useSong.getState().edited).toBe(true);
  expect(lastSave()?.songEdited).toBe(true);
});

it('generating a fresh song drops the locks placed on the old one', async () => {
  // A lock names a section index and a part. A fresh generation has neither the
  // same sections nor the same clips, so a kept lock would pin whatever landed
  // at that index — which is not what the producer pinned.
  invoke.mockResolvedValue(song());
  useSong.setState({ song: song(), locks: ['0:drums'] });

  await useSong.getState().generate({
    styleId: 'trap',
    seed: '9',
    pins: {} as never,
    mood: null,
    base: null,
    complexity: 'authored',
  });

  expect(useSong.getState().locks).toEqual([]);
});

it('a section lock covers every clip in it, and unlocks as one', () => {
  useSong.setState({ song: song() });
  useSong.getState().toggleSectionLock(1);
  expect(useSong.getState().locks).toEqual(['1:drums']);

  useSong.getState().toggleSectionLock(1);
  expect(useSong.getState().locks).toEqual([]);
});

it('a row lock covers that part in every section that plays it', () => {
  useSong.setState({ song: song() });
  useSong.getState().toggleRowLock('drums');
  expect(useSong.getState().locks.sort()).toEqual(['0:drums', '1:drums']);
});

// ---------------------------------------------------------------------------
// Arrangement undo (TASK-063B). One stack, shared with the session.
// ---------------------------------------------------------------------------

it('Ctrl+Z steps an arrangement edit back', () => {
  armedWith();
  useSong.getState().resize(0, 8);
  expect(useSong.getState().song?.sections[0].bars).toBe(8);

  useSession.getState().undo();

  expect(useSong.getState().song?.sections[0].bars).toBe(4);
  // And the section after it moved back with it, because a resize retiles.
  expect(useSong.getState().song?.sections[1].startBar).toBe(4);
});

it('an undone arrangement edit is redoable', () => {
  armedWith();
  useSong.getState().resize(0, 8);
  useSession.getState().undo();
  useSession.getState().redo();

  expect(useSong.getState().song?.sections[0].bars).toBe(8);
});

it('undoing an arrangement edit does not step the session back', () => {
  // ⛔ The regression that made this shortcut a deliberate no-op in the first
  // place: Ctrl+Z on the Song tab used to revert a seed keystroke or a pin made
  // on another tab while the arrangement stayed exactly as it was — damage the
  // producer would only find later, somewhere else.
  armedWith();
  useSession.setState({ seed: '4242' });
  useSong.getState().resize(0, 8);

  useSession.getState().undo();

  expect(useSong.getState().song?.sections[0].bars).toBe(4);
  expect(useSession.getState().seed).toBe('4242');
});

it('a restore is not recorded as a fresh edit', () => {
  // ⛔ Without the `applying` guard in `noteDocumentChange`, restoring the song
  // would record the restored state as a new step — so every Ctrl+Z would push
  // an entry as it popped one and the stack could never be walked out of.
  armedWith();
  useSong.getState().resize(0, 8);
  useSong.getState().resize(0, 12);

  useSession.getState().undo();
  useSession.getState().undo();

  expect(useSong.getState().song?.sections[0].bars).toBe(4);
  expect(canUndo(useHistory.getState())).toBe(false);
  expect(canRedo(useHistory.getState())).toBe(true);
});

it('two arrangement edits in quick succession are two undo steps', () => {
  // ⛔ `song` is DISCRETE for the same reason `mutedLanes` is. Coalescing suits
  // a value being typed toward; deleting a clip and then cloning a section
  // inside 600 ms is two intentions, and merging them would make the state
  // between them unreachable.
  armedWith();
  useSong.getState().resize(0, 8);
  useSong.getState().resize(1, 12);

  useSession.getState().undo();
  expect(useSong.getState().song?.sections[1].bars).toBe(8);
  expect(useSong.getState().song?.sections[0].bars).toBe(8);

  useSession.getState().undo();
  expect(useSong.getState().song?.sections[0].bars).toBe(4);
});

// ---------------------------------------------------------------------------
// Playback (TASK-072). The arrangement reaching the audio thread.
// ---------------------------------------------------------------------------

/** The last `arm_song` request, or null. */
function lastArm(): { song: Song; loopSection: number | null; parts: Part[] | null } | null {
  const calls = invoke.mock.calls.filter(([command]) => command === 'arm_song');
  const last = calls.at(-1);
  return last
    ? (last[1] as { request: { song: Song; loopSection: number | null; parts: Part[] | null } })
        .request
    : null;
}

it('every arrangement edit re-arms what is playing', () => {
  // ⛔ A resize retiles the whole song, so the clip already on the audio thread
  // describes bars that have moved. Without this the producer goes on hearing
  // the arrangement they had before while the timeline draws the one they made.
  useSong.setState({ song: song() });
  useSong.getState().resize(0, 8);

  expect(lastArm()?.song.sections[0].bars).toBe(8);
});

it('undo re-arms the arrangement it restored', () => {
  armedWith();
  useSong.getState().resize(0, 8);
  useSession.getState().undo();

  expect(lastArm()?.song.sections[0].bars).toBe(4);
});

it('a section loop is sent as an index, and toggles off', () => {
  useSong.setState({ song: song() });
  useSong.getState().setLoopSection(1);
  expect(lastArm()?.loopSection).toBe(1);

  useSong.getState().setLoopSection(null);
  expect(lastArm()?.loopSection).toBeNull();
});

it('the whole record sends no part filter at all', () => {
  // `null` rather than "every part", so the common case takes the engine's
  // whole-song path and there is one less list that can be wrong.
  useSong.setState({ song: song() });
  useSong.getState().armSong();
  expect(lastArm()?.parts).toBeNull();
});

it('solo wins over mute', () => {
  // ⛔ What every DAW does, and not merely a convention: a producer soloing the
  // drums has usually muted something earlier and forgotten, and making them
  // undo that first would mean solo sometimes did nothing while the row stayed
  // lit saying otherwise.
  useSong.setState({ song: song(), mutedParts: ['drums'] });
  useSong.getState().togglePartSolo('drums');

  expect(lastArm()?.parts).toEqual(['drums']);
});

it('muting a part leaves the others playing', () => {
  useSong.setState({ song: song() });
  useSong.getState().togglePartMute('drums');
  expect(lastArm()?.parts).toEqual([]);
});

it('generating a fresh song drops the loop, because the indices moved', async () => {
  invoke.mockResolvedValue(song());
  useSong.setState({ song: song(), loopSection: 1 });

  await useSong.getState().generate({
    styleId: 'trap',
    seed: '9',
    pins: {} as never,
    mood: null,
    base: null,
    complexity: 'authored',
  });
  expect(useSong.getState().loopSection).toBeNull();
});

// ---------------------------------------------------------------------------
// Timeline interactions (TASK-071): audition, re-roll, drill-in.
// ---------------------------------------------------------------------------

it('an audition arms that one cell, looping its section', () => {
  useSong.setState({ song: song() });
  useSong.getState().auditionClip({ sectionIndex: 1, part: 'drums' });

  expect(lastArm()?.loopSection).toBe(1);
  expect(lastArm()?.parts).toEqual(['drums']);
});

it('an audition overrides the loop and the solo while it lasts, then gives them back', () => {
  // ⛔ It has to override both, or "audition this cell" would play the cell
  // *and* whatever else was soloed. And it has to hand them back, or ending an
  // audition would silently drop settings the producer never touched.
  useSong.setState({ song: song(), loopSection: 0, soloParts: ['melody'] });
  useSong.getState().auditionClip({ sectionIndex: 1, part: 'drums' });
  expect(lastArm()?.loopSection).toBe(1);
  expect(lastArm()?.parts).toEqual(['drums']);

  useSong.getState().stopAudition();
  expect(lastArm()?.loopSection).toBe(0);
  expect(lastArm()?.parts).toEqual(['melody']);
});

it('auditioning the same cell twice is the way out of it', () => {
  useSong.setState({ song: song() });
  useSong.getState().auditionClip({ sectionIndex: 1, part: 'drums' });
  useSong.getState().auditionClip({ sectionIndex: 1, part: 'drums' });
  expect(useSong.getState().audition).toBeNull();
});

it('drilling into a clip opens it in its own editor', () => {
  useSong.setState({ song: song() });
  useSong.getState().drillInto({ sectionIndex: 0, part: 'drums' });

  expect(useSession.getState().patterns.drums?.id).toBe('a');
  // ⛔ Marked edited, and it is not a guess: a clip lifted out of an arrangement
  // is not what *this session's* seed produces. Left false, the next save would
  // drop it and the next Generate would silently replace it.
  expect(useSession.getState().edited).toBe(true);
  expect(useSong.getState().drillPatternId).toBe('a');
});

it('editing a drilled-in clip writes back to the song', () => {
  // The roadmap's own requirement, and the half a drill-in is worthless
  // without: the producer edits in the roll and the arrangement has the edit.
  useSong.setState({ song: song() });
  useSong.getState().drillInto({ sectionIndex: 0, part: 'drums' });

  const open = useSession.getState().patterns.drums;
  expect(open).toBeDefined();
  useSession.getState().editPattern({ ...open!, bars: 8 });

  expect(useSong.getState().song?.patterns.a?.bars).toBe(8);
  expect(useSong.getState().edited).toBe(true);
});

it('generating on a part tab after drilling in does not overwrite the song', () => {
  // ⛔ A fresh generation is a *new* clip, not an edit of the song's — writing
  // it back would drop a whole section's arrangement into the timeline without
  // anybody asking for it. The id is what tells the two apart.
  useSong.setState({ song: song() });
  useSong.getState().drillInto({ sectionIndex: 0, part: 'drums' });

  const open = useSession.getState().patterns.drums;
  useSession.getState().editPattern({ ...open!, id: 'something-else', bars: 8 });

  expect(useSong.getState().song?.patterns.a?.bars).toBe(4);
});

it('closing a drill-in stops the write-back', () => {
  useSong.setState({ song: song() });
  useSong.getState().drillInto({ sectionIndex: 0, part: 'drums' });
  useSong.getState().closeDrill();

  const open = useSession.getState().patterns.drums;
  useSession.getState().editPattern({ ...open!, bars: 8 });
  expect(useSong.getState().song?.patterns.a?.bars).toBe(4);
});

// ---------------------------------------------------------------------------
// What the correctness review found. Every one of these was a live defect.
// ---------------------------------------------------------------------------

it('the snapshot taken on an artist change does not carry the old artist’s song', () => {
  // ⛔ **The worst of them.** `song.ts` imports `session.ts`, so the history
  // recorder is registered first and runs first for the same `set` — before the
  // subscriber that clears the arrangement on an artist change. It therefore
  // filed the outgoing artist's whole record under the incoming artist's id, and
  // one Ctrl+Z brought it back under a different name and saved it there.
  //
  // Fixed by comparing `song.artistId` to `selectedId` in `snapshotOf` rather
  // than by reordering subscribers, so registration order cannot break it again.
  const current = song(); // artistId: 'trap'
  useSong.setState({ song: current, edited: true });
  useSession.setState({ selectedId: 'trap' });
  useHistory.getState().arm({
    selectedId: 'trap',
    seed: '7',
    songSeed: '7',
    seedPinned: true,
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
    complexity: 'authored' as const,
    patterns: {},
    editedParts: [],
    edited: false,
    mood: null,
    base: null,
    audioEnabled: true,
    mutedLanes: [],
    soloedLanes: [],
    lockedLanes: [],
    partsOff: [],
    song: current,
    songEdited: true,
    // The kit the snapshot was taken over (TASK-050A). Empty: these fixtures
    // are about the session fields, and a kit belongs to the plugin.
    oneShots: {},
  });

  // The artist moves. The song store is cleared by its own subscriber, but the
  // history recorder has already run.
  useSession.setState({ selectedId: 'uk-drill' });

  const recorded = useHistory.getState().present?.state;
  expect(recorded?.selectedId).toBe('uk-drill');
  expect(recorded?.song).toBeNull();
  expect(recorded?.songEdited).toBe(false);
});

it('an edit to a drilled-in clip does not re-arm the whole song', () => {
  // ⛔ It runs on the *part* tab with the roll on screen, and the clip has
  // already reached the audio thread through the session's own subscriber.
  // Arming the arrangement here put the whole record on the transport while the
  // roll drew four bars — and again on every subsequent note edit.
  useSong.setState({ song: song() });
  useSong.getState().drillInto({ sectionIndex: 0, part: 'drums' });
  invoke.mockClear();

  const open = useSession.getState().patterns.drums;
  useSession.getState().editPattern({ ...open!, bars: 8 });

  expect(invoke.mock.calls.filter(([c]) => c === 'arm_song')).toHaveLength(0);
  // The write-back itself still happened.
  expect(useSong.getState().song?.patterns.a?.bars).toBe(8);
});

it('a drilled-in clip is never written back into a different song', () => {
  // ⛔ `pattern_id` is `{model}-{section}-{part}` and carries **no seed**, so
  // two generations of one artist reuse it. Drill in, Generate again, edit a
  // note on the part tab, and song #1's clip replaced song #2's everywhere.
  useSong.setState({ song: song() });
  useSong.getState().drillInto({ sectionIndex: 0, part: 'drums' });

  // A second generation: same clip ids, different song id.
  const next = { ...song(), id: 'trap-song-99', seed: '99' };
  useSong.setState({ song: next });

  const open = useSession.getState().patterns.drums;
  useSession.getState().editPattern({ ...open!, bars: 8 });

  expect(useSong.getState().song?.patterns.a?.bars).toBe(4);
});

it('merely looking at a clip is not an edit', () => {
  // `drillInto` hands the editor the *same object* that is in the store, so the
  // write-back would otherwise rebuild the song into an equal-but-new one, mark
  // it edited and push an undo step that changes nothing on screen — and put a
  // never-edited arrangement into the project file.
  useSong.setState({ song: song(), edited: false });
  useSong.getState().drillInto({ sectionIndex: 0, part: 'drums' });

  expect(useSong.getState().edited).toBe(false);
});

it('a form picked for one artist does not follow to the next', () => {
  // ⛔ A form index means a different form for a different artist, and one past
  // the end of what they author makes every Generate fail — with no control on
  // screen to clear it, because the picker lives inside the timeline and the
  // timeline only mounts once a song exists. `mood` is cleared for exactly this
  // reason and this was not.
  useSession.setState({ selectedId: 'trap' });
  useSong.setState({ song: song(), structure: 1 });

  useSession.setState({ selectedId: 'uk-drill' });

  expect(useSong.getState().structure).toBeNull();
});

it('generating a fresh song drops an audition placed on the old one', async () => {
  // An audition names a section index *and* overrides both the loop and the
  // part filter, so a kept one armed the brand-new song looping one cell of a
  // section the producer never touched.
  invoke.mockResolvedValue(song());
  useSong.setState({ song: song(), audition: { sectionIndex: 1, part: 'drums' } });

  await useSong.getState().generate({
    styleId: 'trap',
    seed: '9',
    pins: {} as never,
    mood: null,
    base: null,
    complexity: 'authored',
  });
  expect(useSong.getState().audition).toBeNull();
});

// ---------------------------------------------------------------------------
// What the workflow-backed /code-review found. All 15 were verified defects.
// ---------------------------------------------------------------------------

/** The Song tab is what `armSong` now gates on. */
function onSongTab() {
  useUi.setState({ activeTab: 'song' });
}

it('cloning a section moves the locks, loop and audition along with it', () => {
  // ⛔ `cloneSection` splices the copy in at `index + 1`, renumbering every
  // section after it. `locks` are `"${sectionIndex}:${part}"`, `loopSection` is
  // an index and `audition` holds one — so a lock placed on the hook drew on a
  // different section afterwards, and pressing R on the hook sent an empty
  // locked list and regenerated the very clips the padlock said were pinned.
  onSongTab();
  useSong.setState({
    song: song(),
    locks: ['1:drums'],
    loopSection: 1,
    audition: { sectionIndex: 1, part: 'drums' },
  });

  useSong.getState().clone(0);

  expect(useSong.getState().locks).toEqual(['2:drums']);
  expect(useSong.getState().loopSection).toBe(2);
  expect(useSong.getState().audition?.sectionIndex).toBe(2);
});

it('a re-roll keeps the anchor, so the next R hits the same section', async () => {
  // Every keyboard gesture reads `anchor ?? 0`, so clearing it made the second
  // press of R re-roll the intro instead of the section being worked on — and
  // the next Ctrl+D and Ctrl+V land there too.
  onSongTab();
  const current = song();
  invoke.mockResolvedValue(current);
  useSong.setState({ song: current, anchor: 1 });

  await useSong.getState().reroll(1, null);

  expect(useSong.getState().anchor).toBe(1);
});

it('a re-roll drops the clipboard and the drill-in, because the engine prunes', async () => {
  // ⛔ `prune_patterns` deletes every clip no section names — exactly what a cut
  // clip is, and what a drilled-in clip becomes when its section is re-rolled.
  // Kept, Ctrl+V pasted nothing silently and a later note edit was written back
  // under an id nothing referenced, then persisted as an orphan.
  onSongTab();
  const current = song();
  invoke.mockResolvedValue(current);
  useSong.setState({
    song: current,
    clipboard: { sectionIndex: 0, clips: [] } as never,
    drillPatternId: 'a',
    drillSongId: current.id,
  });

  await useSong.getState().reroll(0, null);

  expect(useSong.getState().clipboard).toBeNull();
  expect(useSong.getState().drillPatternId).toBeNull();
});

it('generating drops the clipboard, which holds ids the new song also has', async () => {
  // `pattern_id` carries no seed, so `trap-hook-melody` exists in the new song
  // too and the paste guard passes. With `anchor` cleared it would land on
  // section 0 — a clip nobody copied, on a section nobody targeted.
  onSongTab();
  invoke.mockResolvedValue(song());
  useSong.setState({ song: song(), clipboard: { sectionIndex: 5, clips: [] } as never });

  await useSong.getState().generate({
    styleId: 'trap',
    seed: '9',
    pins: {} as never,
    mood: null,
    base: null,
    complexity: 'authored',
  });

  expect(useSong.getState().clipboard).toBeNull();
});

it('generating drops a solo, which the new song may have no row for', async () => {
  // A solo on a part the new form does not play arms an empty clip and plays
  // silence over a timeline full of clips — and `partsInUse` does not draw that
  // row, so there is no lit badge on screen to turn off.
  onSongTab();
  invoke.mockResolvedValue(song());
  useSong.setState({ song: song(), soloParts: ['counter'], mutedParts: ['melody'] });

  await useSong.getState().generate({
    styleId: 'trap',
    seed: '9',
    pins: {} as never,
    mood: null,
    base: null,
    complexity: 'authored',
  });

  expect(useSong.getState().soloParts).toEqual([]);
  expect(useSong.getState().mutedParts).toEqual([]);
});

it('the arrangement is not armed while a part tab is showing', () => {
  // ⛔ There is one schedule and the visible tab decides whose it is. Undo runs
  // on every tab now, and a project restore runs on whichever tab is open — so
  // without the gate an undo taken over the drum grid put the whole record on
  // the transport while the grid drew four bars.
  useUi.setState({ activeTab: 'drums' });
  useSong.setState({ song: song() });

  useSong.getState().armSong();

  expect(invoke.mock.calls.filter(([c]) => c === 'arm_song')).toHaveLength(0);
});

it('an arrangement-only undo reaches the project file', () => {
  // ⛔ Neither persist subscriber fires for it: the SAVED_FIELDS one returns
  // early when no session field changed, the pattern one when the clip is
  // unchanged. So a producer undid a resize, watched it snap back, closed the
  // project and reopened it to find the resize still there.
  // ⚠ Two resizes and one undo, deliberately: undoing all the way back to the
  // *unedited* baseline correctly saves no song at all — that absence is what
  // makes a reopened project regenerate from the seed. The bug is about a
  // restore never reaching the file, so the case has to land on a state that
  // still has one.
  onSongTab();
  armedWith();
  useSong.getState().resize(0, 8);
  useSong.getState().resize(0, 12);
  flushSave();
  invoke.mockClear();

  useSession.getState().undo();
  flushSave();

  const saved = lastSave();
  expect(saved).not.toBeNull();
  expect((saved?.song as Song).sections[0].bars).toBe(8);
});

it('a preset load does not leave a snapshot naming the arrangement it deleted', () => {
  // ⛔ `put()` applies the preset in one `set` — which fires the history
  // recorder — and only afterwards clears the arrangement the preset does not
  // carry. The entry it filed therefore named a record no longer on screen, so
  // one Ctrl+Y resurrected it alongside the preset's pins: a state nobody had
  // been in. `amend` corrects that entry without pushing a second one.
  onSongTab();
  armedWith();
  useSong.getState().resize(0, 8);

  useSession.getState().applyPreset({
    selectedId: 'trap',
    seed: '99',
    songSeed: '99',
    bars: 4,
    pins: null,
  });

  // The arrangement is gone from the screen…
  expect(useSong.getState().song).toBeNull();
  // …and from the entry that is current, so redo cannot bring it back.
  expect(useHistory.getState().present?.state.song).toBeNull();
});

/**
 * Revealing the stems on a Song-Mode generation (TASK-143, cause 1).
 *
 * ⛔⛔ **Mike, 2026-08-06:** *"sometimes when i have the song generator on and
 * the stems panel is supposed be shown it doesn't show it, but then when i press
 * view all panels it shows the stems panel that should already be showing."*
 * `session.ts`'s reveal subscriber is gated on `useSession.patterns`, and
 * generating an arrangement writes only to this store — so the panel appeared
 * only when a per-part pattern happened to exist already. That is the
 * *"sometimes"*. `ui.test.ts` covers the other cause, the unmounted rail.
 */
it('asks for the stems panel when an arrangement arrives, with no pattern behind it', () => {
  useUi.setState({ stemsRevealed: false, rightRailOpen: false });
  // ⛔ Empty on purpose: this is the case the session subscriber cannot see.
  expect(useSession.getState().patterns).toEqual({});

  useSong.setState({ song: song() });

  expect(useUi.getState().stemsRevealed).toBe(true);
  expect(useUi.getState().sections.stems).toBe(true);
});

it('does not re-ask once the producer has switched the rail away', () => {
  // ⚠ The subscriber fires on `song` gaining a value, not on every write, and
  // `revealStems` is one-shot as well. Either alone would be enough; both
  // together are what stop this being a panel policy that reasserts itself on
  // every edit of the arrangement.
  useUi.setState({ stemsRevealed: false });
  useSong.setState({ song: song() });
  useUi.getState().showSection('session');
  expect(useUi.getState().sections.stems).toBe(false);

  useSong.setState({ song: null });
  useSong.setState({ song: song() });

  expect(useUi.getState().sections.stems).toBe(false);
});
