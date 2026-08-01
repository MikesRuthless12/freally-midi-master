/**
 * Canned IPC responses for running the UI without a Rust backend.
 *
 * Used by Playwright and by `vite dev` in a plain browser. This is a test
 * fixture, not a second implementation: it returns the smallest response that
 * lets the UI render, and an unknown command is a loud failure rather than a
 * silent `undefined` — a mock that quietly answers everything hides exactly the
 * bugs E2E exists to catch.
 */

import type { InvokeArgs } from './ipc';
import type { Note, Pattern, RosterSummary, SessionDefaults } from './ipc-types';

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
    const request = (args as { request?: { styleId?: string; bars?: number; seed?: string } })
      ?.request;
    const bars = request?.bars ?? 4;
    const ppq = 960;
    const note = (startTick: number, pitch: number, vel: number): Note => ({
      startTick,
      lenTicks: ppq / 4,
      pitch,
      vel,
    });

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
      id: `${request?.styleId ?? 'mock'}-mock`,
      part: 'drums',
      artistId: request?.styleId ?? 'mock',
      // The seed is echoed back so the chip shows what was used, and a fixed
      // one when none was asked for keeps the fixture reproducible.
      seed: request?.seed && request.seed !== '' ? request.seed : '424242',
      bars,
      bpm: 140,
      timeSigNum: 4,
      timeSigDen: 4,
      keyRoot: 6,
      scale: 'natural_minor',
      lanes: [
        { lane: 'kick', notes: kick },
        { lane: 'snare', notes: snare },
        { lane: 'closedHat', notes: hat },
      ],
      ppq,
    };
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

  // The standalone's own transport (TASK-041T).
  //
  // ⛔ **Rejected, because the plugin rejects them.** `playback_status` above
  // reports this shell as not-the-standalone, and `editor.rs` treats a
  // `transport_play` arriving in that state as the page and the plugin
  // disagreeing about which shell they are in — an error rather than a no-op.
  // A mock that resolved them would keep the e2e suite green through exactly
  // the wiring bug the bridge refuses in order to catch.
  transport_play: () => {
    throw new Error('the host owns the transport — press play in your DAW');
  },
  transport_pause: () => {
    throw new Error('the host owns the transport — press play in your DAW');
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
