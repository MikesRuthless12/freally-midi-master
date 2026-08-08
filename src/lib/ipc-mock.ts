/**
 * Canned IPC responses for running the UI without a Rust backend.
 *
 * Used by Playwright and by `vite dev` in a plain browser. This is a test
 * fixture, not a second implementation: it returns the smallest response that
 * lets the UI render, and an unknown command is a loud failure rather than a
 * silent `undefined` — a mock that quietly answers everything hides exactly the
 * bugs E2E exists to catch.
 */

import { ALL_LANES } from '../state/kit';
import type { InvokeArgs } from './ipc';
import type {
  Note,
  Part,
  Pattern,
  PatternRef,
  RosterSummary,
  Section,
  SectionKind,
  SessionDefaults,
  Song,
} from './ipc-types';

type Handler = (args?: InvokeArgs) => unknown;

const handlers: Record<string, Handler> = {
  // Exactly the shape `app_info` returns in plugin/src/bridge.rs — no more, no
  // fewer. It used to omit `arch` and invent two fields the command has never
  // returned, so the About pane rendered "mock / undefined" here and correctly
  // in the real app: a fixture that disagrees with the DTO tests the fixture.
  app_info: () => ({
    version: '0.0.0-mock',
    platform: 'mock',
    arch: 'mock',
  }),

  // The roster, as the real command returns it: two genres and one artist over
  // one of them, which is enough shape for search and the tier badges without
  // pretending to be the shipped dataset.
  // Typed against the generated DTO on purpose: `tsc` then fails if the Rust
  // struct gains or renames a field and this fixture does not follow. An
  // untyped mock that disagrees with the real command tests the fixture — this
  // repo has shipped that bug before (see `app_info` above).
  roster_summary: (): RosterSummary => ({
    datasetVersion: '0.0.0-mock',
    entries: [
      {
        id: 'trap',
        name: 'Trap',
        aliases: [],
        type: 'genre',
        tier: 'standard',
        genres: ['trap'],
        relatedGenres: [],
        era: '2010s',
      },
      {
        id: 'uk-drill',
        name: 'UK Drill',
        aliases: ['drill'],
        type: 'genre',
        tier: 'standard',
        genres: ['drill'],
        relatedGenres: [],
        era: '2018-',
      },
      {
        id: 'mock-artist',
        name: 'Mock Artist',
        aliases: ['mock'],
        type: 'artist',
        tier: 'flagship',
        genres: ['trap'],
        // The one artist relates to one of the two genres, so the roster's
        // cross-filter has something real to narrow in the mock and under
        // Playwright — an all-empty fixture would exercise only the
        // uncurated-dataset path, which is the one that does nothing.
        relatedGenres: ['trap'],
        era: null,
      },
    ],
    problems: [],
  }),

  // There is no DAW behind a browser, so there is no project tempo to follow
  // and the artist's own value stands. `null` rather than a number: reporting
  // a tempo nothing is running at is the readout-that-lies failure the session
  // chips exist to avoid.
  host_session: () => ({
    tempo: null,
    timeSigNum: 4,
    timeSigDen: 4,
    playing: false,
  }),

  // What a style asks for, for the session chips. The key list leads with F♯
  // and the scale list with natural minor, which is what `generate_pattern`
  // below returns — a fixture whose chips disagree with its own pattern would
  // make a real mismatch impossible to see.
  session_defaults: (): SessionDefaults => ({
    bpm: 140,
    keys: ['F#', 'C#', 'G#'],
    scales: ['natural_minor', 'phrygian'],
    swing: { grid: 'sixteenth', amount: 0.54 },
    halfTime: true,
  }),

  // Generation. A real four-bar pattern rather than an empty one, because the
  // grid is the thing under test: kick on every beat, a backbeat snare, and
  // straight 16th hats is enough for a spec to count cells and know the
  // rendering is wired, without this file becoming a second drum engine.
  generate_pattern: (args): Pattern => {
    const request = (
      args as {
        request?: {
          styleId?: string;
          bars?: number;
          seed?: string;
          songSeed?: string;
          part?: Part;
        };
      }
    )?.request;
    const bars = request?.bars ?? 4;
    const part: Part = request?.part ?? 'drums';
    const ppq = 960;
    // ⛔ **`vel` is spread and `modelVel` is what the model asked for**, because
    // that is what the engine hands back: `humanize` multiplies the tier value
    // by a random factor and keeps the original beside it (TASK-041V). A fixture
    // where the two were equal would let the velocity lane's reset pass while
    // doing nothing, which is the one thing that gesture must not do. Derived
    // from the note's own position rather than a random source, because a
    // fixture that moves is not a fixture.
    const note = (startTick: number, pitch: number, vel: number): Note => ({
      startTick,
      lenTicks: ppq / 4,
      pitch,
      vel: Math.min(127, Math.max(1, vel + (((startTick / 120 + pitch) % 7) - 3))),
      modelVel: vel,
    });

    const shell = {
      id: `${request?.styleId ?? 'mock'}-mock`,
      artistId: request?.styleId ?? 'mock',
      // The seed is echoed back so the chip shows what was used, and a fixed
      // one when none was asked for keeps the fixture reproducible.
      seed: request?.seed && request.seed !== '' ? request.seed : '424242',
      // ⛔ **Echoes the requested *song* seed, not the take** (TASK-141), which
      // is what the real bridge does. Mirroring `seed` here would have made the
      // carry look like it worked no matter what the page sent — the mock
      // agreeing with a bug is worse than no mock at all.
      songSeed:
        request?.songSeed && request.songSeed !== ''
          ? request.songSeed
          : request?.seed && request.seed !== ''
            ? request.seed
            : '424242',
      bars,
      bpm: 140,
      timeSigNum: 4,
      timeSigDen: 4,
      keyRoot: 6,
      scale: 'natural_minor' as const,
      ppq,
    };

    // ⛔ A melodic part answers in *its own lane*, because that is the one thing
    // the piano roll reads (`notes.ts::laneOf`). A fixture that returned drum
    // lanes for a melody request would draw an empty roll and look like the
    // editor was broken rather than the fixture.
    if (part !== 'drums') {
      // An eighth-note figure walking a minor pentatonic around F♯3 — enough
      // shape for a spec to move, resize and delete a real note without this
      // file becoming a second melody generator.
      const steps = [0, 3, 5, 7, 10, 7, 5, 3];
      const melodic: Note[] = [];
      for (let bar = 0; bar < bars; bar += 1) {
        for (let step = 0; step < steps.length; step += 1) {
          melodic.push(
            note(bar * ppq * 4 + step * (ppq / 2), 54 + steps[step], step === 0 ? 108 : 84),
          );
        }
      }
      return { ...shell, part, lanes: [{ lane: part, notes: melodic }] };
    }

    const kick: Note[] = [];
    const snare: Note[] = [];
    const hat: Note[] = [];
    for (let bar = 0; bar < bars; bar += 1) {
      const start = bar * ppq * 4;
      for (let beat = 0; beat < 4; beat += 1) {
        kick.push(note(start + beat * ppq, 36, 110));
        if (beat % 2 === 1) snare.push(note(start + beat * ppq, 38, 118));
      }
      for (let step = 0; step < 16; step += 1) {
        hat.push(note(start + step * (ppq / 4), 42, step % 4 === 0 ? 100 : 72));
      }
    }

    return {
      ...shell,
      part: 'drums',
      lanes: [
        { lane: 'kick', notes: kick },
        { lane: 'snare', notes: snare },
        { lane: 'closedHat', notes: hat },
      ],
    };
  },

  // Song Mode (TASK-065). Built out of the same `generate_pattern` above rather
  // than a second note fixture, so a spec that counts notes in the arrangement
  // view and one that counts them in the roll cannot disagree.
  //
  // The form is `_defaults`' own — intro, verse, hook, verse, hook, outro —
  // with the bar counts `_defaults` authors, because a spec asserting the ruler
  // draws 56 bars has to have a fixture whose bars are knowable by reading it.
  generate_song: (args): Song => {
    const request = (args as { request?: { styleId?: string; seed?: string } })?.request;
    const artistId = request?.styleId ?? 'mock';
    const seed = request?.seed && request.seed !== '' ? request.seed : '424242';

    const form: { kind: SectionKind; bars: number; parts: Part[] }[] = [
      { kind: 'intro', bars: 4, parts: ['melody'] },
      { kind: 'verse', bars: 16, parts: ['drums', 'melody', 'chords'] },
      { kind: 'hook', bars: 8, parts: ['drums', 'melody', 'counter', 'chords'] },
      { kind: 'verse', bars: 16, parts: ['drums', 'melody', 'chords'] },
      { kind: 'hook', bars: 8, parts: ['drums', 'melody', 'counter', 'chords'] },
      { kind: 'outro', bars: 4, parts: ['drums', 'melody'] },
    ];

    const patterns: Record<string, Pattern> = {};
    const sections: Section[] = [];
    let startBar = 0;

    for (const [index, entry] of form.entries()) {
      const refs: Partial<Record<Part, PatternRef>> = {};
      for (const part of entry.parts) {
        // Keyed by section kind, so the two verses share one pattern exactly as
        // `arrange.rs` makes them.
        const patternId = `${artistId}-${entry.kind}-${part}`;
        if (!patterns[patternId]) {
          patterns[patternId] = {
            ...(handlers.generate_pattern({
              request: { styleId: artistId, bars: 4, seed, part },
            }) as Pattern),
            id: patternId,
          };
        }
        refs[part] = { patternId };
      }
      sections.push({
        type: entry.kind,
        startBar,
        bars: entry.bars,
        patterns: refs as Record<Part, PatternRef>,
        // The drop-out sits on whatever runs into a hook, never on the hook.
        dropOutBeats: form[index + 1]?.kind === 'hook' ? 2 : 0,
        decay: entry.kind === 'outro',
        markers: [],
      });
      startBar += entry.bars;
    }

    return {
      id: `${artistId}-song-${seed}`,
      artistId,
      seed,
      bpm: 140,
      keyRoot: 6,
      scale: 'natural_minor',
      sections,
      timeSigNum: 4,
      timeSigDen: 4,
      patterns,
      ppq: 960,
    };
  },

  // Exporting a song to a file (TASK-073). A browser has no native Save As and
  // no filesystem, so the mock reports the shape of a *cancelled* export: the
  // dialog opened and the producer closed it.
  //
  // ⛔ Cancelled rather than done, deliberately. `done` carries a path, and a
  // fixture inventing one would let a spec assert a file was written in a
  // browser that cannot write files — which is the fixture testing itself.
  // Cancelled is the one outcome that is *true* here.
  export_song: () => undefined,
  export_stems: () => undefined,
  export_status: () => ({ state: 'cancelled' }),

  // Stems for the parts on screen (TASK-131F). Cancelled, like the exports
  // above and for the same reason: a browser has no native folder picker, and
  // a fixture reporting `done` would let a spec assert a file was written in a
  // shell that cannot write files.
  export_pattern_stems: () => undefined,

  // Dragging a part out into the DAW (TASK-063C). ⚠ **Reachable but inert**,
  // and both halves of that are deliberate: `bridge_names.rs` asserts every
  // command the page invokes is answered, so these must exist here — and
  // `drag_supported` says `false`, so no handle is ever rendered to call them
  // and a browser cannot pretend to have started an OS drag.
  //
  // ⛔ **`false` is the honest answer, not a convenience.** A browser has no
  // `DoDragDrop`; a fixture saying `true` would let a spec assert the drag
  // works somewhere it never can. The same rule the export fixtures follow.
  drag_supported: () => ({ supported: false }),
  drag_prepare: () => undefined,
  drag_status: () => ({ state: 'idle' }),
  drag_start: () => 'cancelled',
  drag_cancel: () => undefined,

  // The KIT panel (TASK-131B, TASK-136). The shape `kit_state` answers with,
  // for every lane the engine has — because that is what the panel enumerates,
  // and a fixture listing only the eight drum lanes would let the four melodic
  // rows go missing without a spec noticing.
  //
  // ⚠ `snap` is `shipped: false` here because it is `false` in the real kit:
  // the drum generator can write that lane and no shipped pad has ever played
  // it. A fixture that quietly made it `true` would hide the one state the
  // panel exists to be able to show.
  kit_state: () => ({
    id: 'trap-default',
    lanes: ALL_LANES.map((lane) => ({
      lane,
      shipped: lane !== 'snap',
      name: null,
      path: null,
    })),
  }),

  // Assigning one. A browser has no native Open dialog and no filesystem, so
  // the mock reports a *cancelled* assignment for exactly the reason
  // `export_status` above reports a cancelled export: `done` would claim a file
  // was read in a shell that cannot read files, and cancelled is the one
  // outcome that is true here.
  one_shot_assign: () => undefined,
  one_shot_clear: () => undefined,
  one_shot_status: () => ({ state: 'cancelled' }),

  // The sample browser (TASK-132). A browser has no filesystem, so this is a
  // fixture with one library folder and a handful of rows — enough to exercise
  // the listing, the selection and the drag source. ⚠ It does not pretend a
  // dialog can open: `explorer_pick` adds nothing, which is what a shell with
  // no native picker honestly does.
  explorer_state: () => ({
    roots: [{ name: 'Samples', path: '/library/Samples', isDir: true }],
    folder: '/library/Samples',
    parent: null,
    entries: [
      { name: 'Kicks', path: '/library/Samples/Kicks', isDir: true },
      { name: 'clap-01.wav', path: '/library/Samples/clap-01.wav', isDir: false },
      { name: 'kick-808.wav', path: '/library/Samples/kick-808.wav', isDir: false },
    ],
    truncated: false,
    // No native picker in a browser, so a dialog is never open — which is what
    // stops  polling for one.
    picking: false,
  }),
  explorer_pick: () => undefined,
  explorer_remove: () => undefined,
  explorer_open: () => undefined,
  explorer_drop: () => undefined,
  // ⚠ Both bounds per column, like the real command — a fixture returning one
  // amplitude would draw the half-waveform the Rust test exists to refuse, and
  // the mock would be the thing hiding it.
  explorer_waveform: (args?: InvokeArgs) => ({
    path: String((args as { path?: unknown } | undefined)?.path ?? ''),
    name: 'kick-808.wav',
    peaks: Array.from({ length: 64 }, (_, i) => {
      const amplitude = Math.abs(Math.sin(i / 6)) * (1 - i / 80);
      return [-amplitude, amplitude] as [number, number];
    }),
    seconds: 1.5,
  }),

  // The audition voice. No audio thread here, so the position never advances —
  // a mock that animated one would make a broken transport look like a working
  // one, which is the rule this whole file is written to.
  preview_load: () => undefined,
  preview_play: () => undefined,
  preview_pause: () => undefined,
  preview_stop: () => undefined,
  preview_seek: () => undefined,
  preview_loop: () => undefined,
  preview_reverse: () => undefined,
  preview_position: () => ({
    playing: false,
    seconds: 0,
    total: 1.5,
    looping: false,
    reverse: false,
  }),

  // The forms this artist writes, for the structure picker (TASK-070).
  //
  // Two, because the picker only renders with more than one — a model that
  // writes exactly one form has nothing to choose between, and a fixture with
  // one would leave the control untested. They differ in a way a spec can read:
  // the second has a bridge.
  song_structures: () => ({
    structures: [
      ['intro', 'verse', 'hook', 'verse', 'hook', 'outro'],
      ['intro', 'verse', 'hook', 'bridge', 'hook', 'outro'],
    ],
  }),

  // Handing the arrangement to the audio thread (TASK-072). A browser has no
  // audio thread and no `Song::flatten`, and writing a second flattener here
  // would be the "second implementation" this file's header rules out — so this
  // resolves without doing anything, which is the honest answer.
  //
  // ⛔ It is here rather than absent because `mockInvoke` treats an unknown
  // command as a loud failure, and every arrangement edit calls this. Without
  // it the Playwright suite would see a rejected promise on every resize.
  arm_song: () => undefined,

  // Taking whatever is playing off the transport. A browser has no audio
  // thread, so there is nothing to disarm — but it is here rather than absent
  // because leaving the Song tab calls it and an unknown command is a loud
  // failure by design.
  disarm: () => undefined,

  // Re-rolling one section (TASK-067). The real engine regenerates the notes;
  // what a spec can meaningfully assert about it is the *shape* of the result —
  // that the named section's clips are new ones, that every other section is
  // untouched, and that locked parts keep the clip they had. So this mock does
  // exactly that transformation and does not pretend to generate anything.
  //
  // ⛔ Keyed by index, the way `arrange::reroll_section` keys it, because that
  // is the property that stops verse 2 dragging verse 1 with it — and a mock
  // that shared one id would make the spec pass on the bug.
  reroll_section: (args): Song => {
    const request = (
      args as {
        request?: { song?: Song; index?: number; locked?: Part[] };
      }
    )?.request;
    const song = request?.song;
    if (!song) throw new Error('reroll_section needs a song');
    const index = request?.index ?? 0;
    const locked = request?.locked ?? [];
    const section = song.sections[index];
    if (!section) throw new Error(`this song has no section ${index}`);

    const patterns: Record<string, Pattern> = {};
    for (const [id, clip] of Object.entries(song.patterns)) {
      if (clip) patterns[id] = clip;
    }
    const refs: Partial<Record<Part, PatternRef>> = { ...section.patterns };
    for (const [name, reference] of Object.entries(section.patterns)) {
      const part = name as Part;
      if (locked.includes(part)) continue;
      const was = patterns[reference.patternId];
      // A section naming a clip the store does not hold is the dangling
      // reference `song_smf` refuses; leaving it alone is the honest mock.
      if (!was) continue;
      const patternId = `${song.artistId}-${section.type}@${index}-${part}`;
      patterns[patternId] = {
        ...was,
        id: patternId,
        // Something a spec can see, without a second note generator here.
        seed: `${Number(was.seed) + 1}`,
        songSeed: `${Number(was.seed) + 1}`,
      };
      refs[part] = { patternId };
    }

    const sections = song.sections.map((s, i) =>
      i === index ? { ...s, patterns: refs as Record<Part, PatternRef> } : s,
    );
    // Clips nothing names any more go, the way the engine prunes them.
    const live = new Set(
      sections.flatMap((s) => Object.values(s.patterns).map((r) => r.patternId)),
    );
    for (const id of Object.keys(patterns)) {
      if (!live.has(id)) delete patterns[id];
    }
    return { ...song, sections, patterns };
  },

  // The exported bytes. A browser has no `song_to_smf`, and inventing an SMF
  // encoder here would be the "second implementation" this file's header rules
  // out — so it answers with the header every SMF starts with and a length,
  // which is enough for a spec to assert a drag produced *something* and not
  // enough to be mistaken for the real encoder.
  song_smf: () => ({ bytes: [0x4d, 0x54, 0x68, 0x64] }),

  // The keyboard gutter's click-to-audition (TASK-041). A browser has no
  // sampler, so this resolves without sounding anything — which is the honest
  // answer and matches what `auditionNote` already expects to be the common
  // case. It is here rather than absent because `mockInvoke` treats an unknown
  // command as a loud failure, and an audition must never be able to break the
  // page it is decorating.
  audition_note: () => undefined,

  // An edited clip going back to the audio thread (TASK-041). The real command
  // validates and echoes; the browser has no audio thread, so echoing is all
  // there is to do — and echoing rather than returning `undefined` keeps the
  // fixture the same shape as the command, which is what `app_info` above is a
  // cautionary tale about.
  // ⚠ Accepted and forgotten. A browser has no schedule to loop, and the real
  // one keeps this on `Shared` for the audio thread — a fixture holding its own
  // copy would be a second answer to "is looping on" that nothing reconciles.
  transport_loop: () => undefined,

  // ⚠ Answers with the FIRST of the parts handed over, not a merge: the real
  // bridge merges them (TASK-127) and a fixture that reimplemented that would be
  // a second, drifting copy of the rule. What a browser needs is a clip back.
  arm_pattern: (args) =>
    (args as { patterns?: unknown[] } | undefined)?.patterns?.[0] ?? undefined,

  // Scale intervals for the roll's row tinting and folding (TASK-041B). The
  // real command reads `engine::theory::scale_semitones`; the fixture answers
  // for the handful of scales the mock generates in, and falls back to the
  // natural minor it reports in `session_defaults` above rather than inventing
  // one — a fixture whose scale disagrees with its own pattern would make a
  // real mismatch impossible to see.
  scale_pitches: (args) => {
    const scale = (args as { scale?: string } | undefined)?.scale;
    const known: Record<string, number[]> = {
      natural_minor: [0, 2, 3, 5, 7, 8, 10],
      aeolian: [0, 2, 3, 5, 7, 8, 10],
      major: [0, 2, 4, 5, 7, 9, 11],
      phrygian: [0, 1, 3, 5, 7, 8, 10],
      minor_pentatonic: [0, 3, 5, 7, 10],
      chromatic: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    };
    return known[scale ?? ''] ?? known.natural_minor;
  },

  // Playback in a browser: there is no audio thread behind the mock, so
  // `playback_status` reports why rather than pretending there is one. The
  // transport is then honestly disabled, which is what the spec asserts — and
  // it is the same shape the plugin answers with, where the reason is that the
  // host owns the transport rather than that there is no host.
  playback_status: () => ({ standalone: false, reason: 'Playback needs the plugin.' }),
  stop_playback: () => undefined,

  // The transport (TASK-041T). A mock playhead that never moves is honest: the
  // browser has no audio thread to advance it, and the seek below is what the
  // e2e spec drives it with.
  playhead: () => 0,
  seek: () => undefined,
  set_looping: () => undefined,

  // The preview transport (TASK-041T, TASK-138).
  //
  // ⛔ **Still rejected here, but for a different reason since TASK-138, and
  // the old one must not be re-quoted.** It threw *"the host owns the transport
  // — press play in your DAW"* because `editor.rs` refused a hosted
  // `transport_play` outright. It no longer does: the plugin drives its own
  // preview transport in a host now.
  //
  // ⚠ **What is still true is that a BROWSER cannot play** — there is no audio
  // thread behind the mock — which is exactly what `playback_status` above
  // reports, and what keeps `canDriveTransport` false and the button disabled
  // here. A fixture that resolved these would let the suite go green over a
  // page that thinks it is playing something.
  transport_play: () => {
    throw new Error('Playback needs the plugin.');
  },
  transport_pause: () => {
    throw new Error('Playback needs the plugin.');
  },

  // Presets (TASK-P13). The real ones are files the plugin owns; a browser has
  // nowhere to put them, so the mock is a fixture that keeps the panel
  // exercisable in `vite dev` and Playwright. Saving reports back rather than
  // storing, because a mock that pretended to persist would make a broken save
  // look like a working one.
  presets_list: () => [
    { id: 'factory/trap', name: 'Trap', factory: true },
    { id: 'factory/uk-drill', name: 'UK Drill', factory: true },
    { id: 'user/my-beat', name: 'My Beat', factory: false },
  ],
  preset_load: () => ({
    selectedId: 'trap',
    seed: '1404',
    songSeed: '1404',
    bars: 8,
    pins: { bpm: null, keyRoot: null, scale: null, swing: null },
  }),
  preset_save: (args?: InvokeArgs) => ({
    id: 'user/mock',
    name: String((args as { name?: unknown } | undefined)?.name ?? 'Mock'),
    factory: false,
  }),
  preset_delete: () => undefined,
};

export async function mockInvoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  const handler = handlers[command];
  if (!handler) {
    throw new Error(
      `ipc-mock has no handler for "${command}". Add one in src/lib/ipc-mock.ts — ` +
        `silently returning undefined would hide the bug this test exists to catch.`,
    );
  }
  return handler(args) as T;
}
