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
import type { PadTweaks } from '../state/kit';
import type { ExplorerEntry, Favourite } from '../state/explorer';
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

/**
 * Styles saved during this page's life, so the roster can show them.
 *
 * ⚠ Deliberately not persisted. The real store writes to the platform data
 * directory and the browser has none; a fake that survived a reload would be a
 * second implementation of the thing `plugin/src/models.rs` already tests.
 */
const userModels = new Map<
  string,
  { entry: RosterSummary['entries'][number]; model: Record<string, unknown> }
>();

/**
 * Every sample path the page has asked to copy this page load.
 *
 * ⛔ Exposed on `window` so a Playwright spec can assert the **negative** —
 * that saving with the consent box unticked copies nothing. A test that can
 * only see what did happen cannot check a gate.
 */
const copiedSamples: string[] = [];

/** The mock kit's assigned samples, as the plugin would source them. */
const assignedSamplePaths = (): string[] =>
  (handlers.kit_state() as { lanes: { path: string | null }[] }).lanes
    .map((lane) => lane.path)
    .filter((path): path is string => path !== null);

/** Kits saved during this page life, so the panel has something to list. */
const savedKits = new Map<string, { id: string; name: string; lanes: number }>();

/**
 * Samples dropped onto a lane from the browser this page load.
 *
 * ⛔⛔ **Without this the whole browser→pad gesture was untestable, and so it was
 * untested.** `explorer_drop` answered `undefined` and `kit_state` was a constant,
 * so a spec could perform the drag and then had nothing to assert: the pad read
 * the same before and after. That is why the one gesture Mike named first has no
 * coverage anywhere in `e2e/` — not because it was hard to drive, but because the
 * mock could not tell a landed drop from a lost one.
 *
 * ⚠ **Modelling this is not the same as faking a filesystem.** The real
 * `explorer_drop` routes to `OneShots::restore`, which decodes the file and
 * rebuilds the kit; decoding is Rust and is tested there. What crosses the bridge
 * is lane → path, and that much is true in a browser. The mock still refuses to
 * pretend a *dialog* opened (`one_shot_assign` reports cancelled) because that
 * genuinely cannot happen here.
 */
const droppedSamples = new Map<string, string>();

/**
 * Per-pad edits made this page load (TASK-055A, TASK-164).
 *
 * ⚠ Mutable for the same reason [`droppedSamples`] is: a constant would let a
 * spec turn every knob and leave nothing to assert.
 */
const padTweaks = new Map<string, PadTweaks>();

/**
 * What the plugin sends for a pad nobody has edited.
 *
 * ⛔ **A fresh object per call, not a shared constant.** The store spreads what
 * it is given and writes the copy back; handing every lane the same object would
 * make one pad's edit appear on all thirty-seven the moment anything mutated it
 * in place. Cheap, and it removes the whole class.
 *
 * ⚠ **Exported so the component tests build their rows from it too.** They used
 * to hand-write partial `KitLane`s, and a second spelling of "untouched" is how
 * a fixture comes to disagree with the fixture the specs run against — the
 * failure recorded on `droppedSamples` above, one field along.
 */
export const untouchedPad = (): PadTweaks => ({
  gainDb: 0,
  pan: 0,
  semis: 0,
  cents: 0,
  normalize: false,
  trimStart: 0,
  trimEnd: 1,
  fadeInS: 0,
  fadeOutS: 0,
  adsr: { attackMs: 0, decayMs: 0, sustainDb: 0, releaseMs: 0 },
});

/** Files starred this page load (TASK-058C). */
const starred = new Map<string, Favourite>();

/**
 * Files the browser opened this page load (TASK-058), newest first.
 *
 * ⚠ It genuinely mutates, for the reason the favourites map does: a mock that
 * answered a constant would let a spec audition a sample and have nothing to
 * assert. The store, its cap and its per-user file are Rust and are tested
 * there — what a browser can show is the list.
 */
let recent: Favourite[] = [];

/**
 * Every generation this page load has made, by style id (TASK-045B).
 *
 * ⚠ Mutable for the reason `recent` is: the panel that browses these is worth
 * nothing if a spec cannot generate four takes and then find them in it. Typed
 * loosely on purpose — `takes::Take` flattens whatever `state/variations.ts`
 * sends, so restating the shape here would be a third definition of it.
 */
let takes: Record<string, { seed?: string; part?: string }[]> = {};

/**
 * The seed an unpinned Generate comes back with.
 *
 * ⛔⛔ **It has to CHANGE, because the real one does.** `bridge.rs` sends `null`
 * for an unpinned seed and the engine draws a fresh one — that is the whole fix
 * for *"Generate returns the same beat every press"* — and this answered the
 * literal `424242` every time. The disagreement was invisible until the take
 * history was built on `(part, seed)`: three presses produced one take, because
 * as far as the fixture was concerned they *were* the same generation.
 *
 * ⚠ **Counted, not random**, which is what the note it replaces actually wanted:
 * a fixture that moves is not a fixture. The first press of a page load is still
 * `424242`, so `magic-moment.spec.ts` still reads the number it was written
 * against, and every press after it is a different take.
 */
let seedCounter = 424_242;

function nextSeed(): string {
  const seed = String(seedCounter);
  seedCounter += 1;
  return seed;
}

/**
 * Record that a file was opened, the way `recent::note` does.
 *
 * ⛔ **The mock records because the PLUGIN records**, and at the same two
 * commands: `editor::rpc` calls `recent::note` inside `preview_load` for a
 * sample and inside `explorer_midi_split` for a `.mid`. A mock that only
 * answered `recent_list` would show a history that never grew, and the specs
 * would be asserting a fixture rather than the behaviour.
 *
 * ⚠ **The MIDI half was missing on both sides until 2026-08-13** — the page
 * refreshed the history after a split on the strength of a comment claiming the
 * plugin had written the entry, and nothing had.
 */
function noteRecent(path: string) {
  if (path === '') return;
  const name = path.split(/[\\/]/).pop() ?? path;
  const kind: Favourite['kind'] = /\.midi?$/i.test(path) ? 'midi' : 'audio';
  recent = [
    { path, name, kind },
    // Newest first and one entry per file, the rule `recent::note` keeps.
    ...recent.filter((held) => held.path !== path),
  ].slice(0, 30);
}

/**
 * The library folders, which a spec can actually remove.
 *
 * ⚠ Mutable for the reason `droppedSamples` gives: `explorer_remove` answered
 * `undefined` and `explorer_state` returned a literal, so removing a root left
 * the panel reporting it — and any behaviour that depends on a folder no longer
 * being open could not be tested at all.
 */
const libraryRoots: ExplorerEntry[] = [
  { name: 'Samples', path: '/library/Samples', isDir: true, kind: 'dir' },
];

/**
 * What each folder holds — **one fixture, read by both commands**.
 *
 * ⛔ `explorer_state` and `explorer_list` each hard-coded `/library/Samples`, and
 * they had already drifted: only one of them listed `riff.mid`. A spec asserting
 * through the flat listing was asserting against a library that does not exist,
 * and the two-kinds rule could not be exercised through it at all.
 *
 * ⚠ A shallow tree rather than one folder, because the defect the tree was built
 * for — *"you cannot go into those subfolders"* — only appears below the first
 * level.
 */
const libraryRows: Record<string, ExplorerEntry[]> = {
  '/library/Samples': [
    { name: 'Kicks', path: '/library/Samples/Kicks', isDir: true, kind: 'dir' },
    { name: 'clap-01.wav', path: '/library/Samples/clap-01.wav', isDir: false, kind: 'audio' },
    {
      name: 'kick-808.wav',
      path: '/library/Samples/kick-808.wav',
      isDir: false,
      kind: 'audio',
    },
    // ⚠ A `.mid` among the samples: a fixture with none cannot catch a panel
    // that treats the two kinds alike.
    { name: 'riff.mid', path: '/library/Samples/riff.mid', isDir: false, kind: 'midi' },
  ],
  '/library/Samples/Kicks': [
    // ⛔⛔ **`MAX_ENTRIES` rows, because that is the size TASK-058 is about.**
    // *"a 2,000-file folder under 300 ms"* — and 2,000 is not a round number
    // picked for a test, it is `explorer::MAX_ENTRIES`, the most rows the plugin
    // will ever answer for one folder. A fixture of four cannot tell a
    // virtualized tree from the recursive one it replaced, which is exactly how
    // this tree shipped un-virtualized with a full green suite.
    //
    // ⚠ **Nested rather than at the top level**, so the row order the keyboard
    // walk steps through is the one it always was — and because a big folder
    // three levels down is the case that actually hurts: its rows carry the
    // deepest indent and the most ancestors above them.
    { name: 'Loops', path: '/library/Samples/Kicks/Loops', isDir: true, kind: 'dir' },
    { name: 'Vinyl', path: '/library/Samples/Kicks/Vinyl', isDir: true, kind: 'dir' },
    {
      name: 'kick-hard.wav',
      path: '/library/Samples/Kicks/kick-hard.wav',
      isDir: false,
      kind: 'audio',
    },
  ],
  '/library/Samples/Kicks/Vinyl': [
    {
      name: 'kick-dusty.wav',
      path: '/library/Samples/Kicks/Vinyl/kick-dusty.wav',
      isDir: false,
      kind: 'audio',
    },
  ],
  // ⚠ Generated rather than written out: 2,000 literals would be 12,000 lines of
  // fixture nobody reads, and the only thing that matters about them is that
  // there are 2,000 and that a filter can pick one out.
  '/library/Samples/Kicks/Loops': Array.from({ length: 2_000 }, (_, at) => ({
    name: `loop-${String(at).padStart(4, '0')}.wav`,
    path: `/library/Samples/Kicks/Loops/loop-${String(at).padStart(4, '0')}.wav`,
    isDir: false,
    kind: 'audio' as const,
  })),
};

/**
 * Paths the page asked to reveal in the OS file manager.
 *
 * ⛔ Exposed for the same reason `copiedSamples` is: the only thing a browser can
 * prove about a command it cannot perform is that it was *asked for*, with the
 * right path. Opening Explorer is not something a mock may pretend to do.
 */
const revealed: string[] = [];

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
        mine: false,
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
        mine: false,
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
        mine: false,
      },
      {
        // ⛔ **A producer, so the rail's two groups both have someone in them.**
        // The roster splits into "Artists" and "Producers" (2026-08-12) and a
        // heading is only drawn when its group is occupied — so with an
        // artist-only fixture the producer half of that rule was unreachable
        // from any spec, and "the separator is not selectable" could only ever
        // have been asserted about the one heading that happened to render.
        //
        // ⚠ Named so it sorts *before* Mock Artist under a naive codepoint
        // sort but after it alphabetically, which is what makes the A–Z
        // assertion mean something.
        id: 'mock-producer',
        name: 'mock Producer',
        aliases: ['boards'],
        type: 'producer',
        tier: 'standard',
        genres: ['trap'],
        relatedGenres: ['trap'],
        era: null,
        mine: false,
      },
      // Appended rather than sorted in, exactly as `dataset::roster()` does it:
      // the rail decides where a producer's own styles appear, not the loader.
      ...[...userModels.values()].map((saved) => saved.entry),
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
    // ⚠ **A real range around the nominal**, so the detail pane's TASK-158D line
    // is exercised rather than collapsing to the single-tempo case.
    bpmMin: 132,
    bpmMax: 148,
    // ⛔ **Four of five, and the gap is the point.** `parts_of` answers what a
    // model will actually write, and a fixture that claimed all five could not
    // tell a working "does not write" line from one that never renders — which
    // is the half of TASK-158D that stops a producer pressing Generate on an
    // empty tab.
    parts: ['drums', 'chords', 'melody', 'bass'],
    keys: ['F#', 'C#', 'G#'],
    scales: ['natural_minor', 'phrygian'],
    swing: { grid: 'sixteenth', amount: 0.54 },
    halfTime: true,
    moods: ['dark', 'bounce'],
  }),

  // The producer's own styles (TASK-040U). ⛔ **In memory and per page load**,
  // not a fake filesystem: the store, its slug rules and its refusals are Rust
  // and are tested there. What a browser can test is the screen — that saving
  // puts a row in the roster marked as yours, that reopening it shows what was
  // saved, and that deleting takes it away — so the mock does exactly enough to
  // let those be asserted, and no more.
  user_model_save: (args?: InvokeArgs) => {
    const model = (args as { model?: Record<string, unknown> } | undefined)?.model ?? {};
    const id = String(model.id ?? '');
    if (id === '') throw new Error('a model needs an `id`');

    const entry = {
      id,
      name: String(model.name ?? id),
      aliases: [],
      type: 'artist' as const,
      tier: null,
      genres: Array.isArray(model.genres) ? (model.genres as string[]) : [],
      relatedGenres: Array.isArray(model.relatedGenres)
        ? (model.relatedGenres as string[])
        : [],
      era: null,
      mine: true,
    };
    userModels.set(id, { entry, model });
    return entry;
  },

  user_model_delete: (args?: InvokeArgs) => {
    userModels.delete(String((args as { id?: string } | undefined)?.id ?? ''));
    return undefined;
  },

  // The sample-copy consent (TASK-049, on the owner's instruction 2026-08-09).
  // ⚠ **Two handlers, mirroring the two commands**, because the split *is* the
  // gate: a save can never copy, and only an explicit call does. The mock
  // records what it was asked to copy so a spec can assert that an unticked box
  // asks for nothing.
  user_model_sample_cost: () => {
    // ⚠ No `paths` argument, and that is the security fix rather than a
    // simplification: the plugin sources the assignments itself, so the page
    // cannot name a file to be measured.
    const paths = assignedSamplePaths();
    return { count: paths.length, bytes: paths.length * 1_500_000 };
  },

  user_model_copy_samples: () => {
    const paths = assignedSamplePaths();
    copiedSamples.push(...paths);
    return paths.map((_, at) => `copied-${at}.wav`);
  },

  // The read-back (TASK-049). ⛔ **`false` when the style owns nothing**, which
  // is what stops selecting a shipped artist waiting on a loader that was never
  // asked to do anything — the page treats the two answers differently, so a
  // mock that always said `true` would hide that.
  user_model_load_samples: () => copiedSamples.length > 0,

  user_model_export: (args?: InvokeArgs) => {
    const id = String((args as { id?: string } | undefined)?.id ?? '');
    const found = userModels.get(id);
    if (found === undefined) throw new Error(`no model \`${id}\``);
    return JSON.stringify(found.model, null, 2);
  },

  user_model_import: (args?: InvokeArgs) => {
    const text = String((args as { text?: string } | undefined)?.text ?? '');
    return handlers.user_model_save({ model: JSON.parse(text) as Record<string, unknown> });
  },

  // Training (TASK-040T). ⛔ **The floor is echoed, not re-implemented.** The
  // fit, its constraints and the variety gate are `engine/src/fit.rs` and are
  // measured there over a thousand seeds — a browser fixture that pretended to
  // do any of that would be a second, worse trainer. What the page needs from
  // this is the one behaviour it draws: enough kept, or a refusal that names the
  // shortfall.
  user_model_train: (args?: InvokeArgs) => {
    const request = args as
      { id?: string; name?: string; base?: string; kept?: unknown[] } | undefined;
    const kept = request?.kept?.length ?? 0;
    if (kept < 30)
      throw new Error(`${kept} of 30 kept — keep more generations before training`);

    return handlers.user_model_save({
      model: {
        id: request?.id ?? '',
        name: request?.name ?? '',
        extends: [request?.base ?? 'trap'],
        genres: [],
        notes: `Trained on ${kept} kept generations.`,
      },
    });
  },

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
      // The seed is echoed back so the chip shows what was used, and an
      // unpinned press draws a new one — see `nextSeed`.
      seed: request?.seed && request.seed !== '' ? request.seed : nextSeed(),
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
  // ⚠ **One lane carries a producer's own sample**, and it is `kick` rather than
  // `melody` because `kit-panel.spec.ts` asserts `melody` reads "Built in". A
  // fixture with no assignment at all could never exercise the sample-copy
  // consent, which is a gate — and a gate with no test is the thing this
  // codebase keeps writing down.
  // ⚠ **Reads `droppedSamples`, so a drop from the browser actually shows up
  // here.** A dropped path wins over the fixture's own assignment, which is what
  // `restore` does — dropping onto a lane that already carries a sample replaces
  // it rather than being ignored.
  // ⚠ **`tweaks` is present on every row, never null** — the plugin sends its
  // own defaults for a lane nobody has edited, so the page never constructs a
  // `PadTweaks` and there is one owner of what "untouched" means. A fixture
  // that omitted it for unedited lanes would let the page grow a second answer
  // that only the real plugin ever contradicts.
  kit_state: () => ({
    id: 'trap-default',
    lanes: ALL_LANES.map((lane) => {
      const dropped = droppedSamples.get(lane);
      const path = dropped ?? (lane === 'kick' ? 'C:/samples/my-kick.wav' : null);
      return {
        lane,
        shipped: lane !== 'snap',
        name: path === null ? null : (path.split(/[\\/]/).pop() ?? path),
        path,
        tweaks: padTweaks.get(lane) ?? untouchedPad(),
      };
    }),
  }),

  // ⛔⛔ **It genuinely stores, for the reason `droppedSamples` above exists.**
  // A handler answering `undefined` would let a spec drag the whole envelope
  // and have nothing to assert — the pad would read the same before and after,
  // which is exactly how the browser→pad gesture went untested for months. The
  // clamping, the kit rebuild and the audio are Rust and are tested there; what
  // crosses the bridge is lane → tweaks, and that much is true in a browser.
  pad_tweaks_set: (args?: InvokeArgs) => {
    const { lane, tweaks } = (args ?? {}) as { lane?: string; tweaks?: PadTweaks };
    if (lane !== undefined && tweaks !== undefined) padTweaks.set(lane, tweaks);
    return undefined;
  },

  // Assigning one. A browser has no native Open dialog and no filesystem, so
  // the mock reports a *cancelled* assignment for exactly the reason
  // `export_status` above reports a cancelled export: `done` would claim a file
  // was read in a shell that cannot read files, and cancelled is the one
  // outcome that is true here.
  // TASK-050A. The pick rule, the seeding and the threading are Rust and are
  // tested there; what the page needs from this is that the command exists and
  // that a kit with every pad locked never reaches it.
  kit_randomize: () => undefined,

  // Named kits (TASK-051). ⚠ In memory and per page load, like the user
  // models above: the store, its slug rule and its refusals are Rust and are
  // tested there. What a browser can show is the panel.
  kits_list: () => [...savedKits.values()],
  kits_save: (args?: InvokeArgs) => {
    const name = String((args as { name?: string } | undefined)?.name ?? '');
    const id = name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '');
    const summary = { id, name, lanes: 1 };
    savedKits.set(id, summary);
    return summary;
  },
  kits_load: () => undefined,
  kits_rename: () => undefined,
  kits_duplicate: (args?: InvokeArgs) => handlers.kits_save(args),
  kits_delete: (args?: InvokeArgs) => {
    savedKits.delete(String((args as { id?: string } | undefined)?.id ?? ''));
    return undefined;
  },
  one_shot_assign: () => undefined,
  // ⚠ Forgets a dropped path too, or clearing a pad would leave `kit_state`
  // still reporting the sample the producer just removed — the readout-that-lies
  // failure, arriving through the fixture instead of through the product.
  one_shot_clear: (args?: InvokeArgs) => {
    droppedSamples.delete(String((args as { lane?: unknown } | undefined)?.lane ?? ''));
    return undefined;
  },
  one_shot_status: () => ({ state: 'cancelled' }),

  // The sample browser (TASK-132). A browser has no filesystem, so this is a
  // fixture with one library folder and a handful of rows — enough to exercise
  // the listing, the selection and the drag source. ⚠ It does not pretend a
  // dialog can open: `explorer_pick` adds nothing, which is what a shell with
  // no native picker honestly does.
  // ⚠ **`libraryRoots`, not a literal.** `explorer_remove` used to answer
  // `undefined` while this went on reporting the root, so removing a folder
  // changed nothing a spec could see — and "a favourite whose folder is no
  // longer open falls back to the OS" was untestable for exactly that reason.
  // Same defect the drop had, one command over.
  explorer_state: () => ({
    roots: [...libraryRoots],
    folder: libraryRoots.length > 0 ? '/library/Samples' : null,
    parent: null,
    // ⚠ The shared fixture — see `libraryRows`. This used to be a second literal
    // and the two had already drifted.
    entries: libraryRows['/library/Samples'] ?? [],
    truncated: false,
    // No native picker in a browser, so a dialog is never open — which is what
    // stops `addFolder` polling for one.
    picking: false,
    // ⚠ **Empty rather than absent**, so the shape matches `explorer::State`.
    // A browser fixture has no drive to unplug, so nothing here is ever
    // missing — the reconnect state is exercised in `explorer.test.ts`, where
    // the reply can be written directly.
    missing: [],
  }),
  // Starred favourites (TASK-058C). In memory and per page load, like the user
  // models and the saved kits: the store, its bounds and its refusals are Rust
  // and are tested there. What a browser can show is the star and the list.
  // ⚠ It genuinely mutates, for the reason `droppedSamples` gives — a mock that
  // answered a constant would let a spec press the star and have nothing to
  // assert.
  favourites_list: () => [...starred.values()],
  recent_list: () => recent,
  recent_clear: () => {
    recent = [];
    return recent;
  },

  // ── The variation history that survives a restart (TASK-045B) ─────────
  //
  // ⛔ **It genuinely accumulates**, for the reason `recent` does: a mock that
  // answered a constant would let a spec generate four takes and have nothing to
  // assert about the panel that lists them. The cap, the per-style eviction and
  // the per-user file are Rust and are tested there — what a browser can show is
  // that generating puts a row in the list and that choosing one restores it.
  //
  // ⚠ **In memory and per page load.** A reload starts empty here, which is
  // honestly what a browser build can offer: there is no `%APPDATA%` to write to.
  takes_list: () => takes,
  // ⚠ **A batch, as the plugin takes**: one Generate press records a take per
  // part, and a mock that took them one at a time would let a per-take round trip
  // pass here and cost five file rewrites in the real thing.
  takes_note: (args?: InvokeArgs) => {
    const batch = (args as { takes?: { artistId?: string; seed?: string; part?: string }[] })
      ?.takes;
    if (!Array.isArray(batch)) throw new Error('that is not a take');
    for (const take of batch) {
      const style = String(take?.artistId ?? '');
      if (style === '') throw new Error('a take belongs to a style');
      const held = takes[style] ?? [];
      // ⛔ Idempotent on `(part, seed)`, the rule `takes::note` keeps: recalling
      // a take regenerates it, and without this stepping backwards would write
      // new history on every step.
      if (!held.some((one) => one.seed === take.seed && one.part === take.part)) {
        takes[style] = [...held, take];
      }
    }
    // ⚠ An acknowledgement, as the plugin's is: `takes_list` is what answers the
    // history, and handing it back on every press would serialize a list nothing
    // on screen is showing.
    return undefined;
  },
  takes_clear: () => {
    takes = {};
    return takes;
  },
  favourites_add: (args?: InvokeArgs) => {
    const path = String((args as { path?: unknown } | undefined)?.path ?? '');
    const name = path.split(/[\\/]/).pop() ?? path;
    starred.set(path, {
      path,
      name,
      kind: /\.midi?$/i.test(path) ? 'midi' : 'audio',
    });
    return [...starred.values()];
  },
  favourites_remove: (args?: InvokeArgs) => {
    starred.delete(String((args as { path?: unknown } | undefined)?.path ?? ''));
    return [...starred.values()];
  },
  // ⛔ A browser cannot open Windows Explorer, and a mock that pretended it had
  // would make a broken reveal look like a working one. Recorded so a spec can
  // assert the *page* asked, which is its half of the contract.
  favourites_reveal: (args?: InvokeArgs) => {
    revealed.push(String((args as { path?: unknown } | undefined)?.path ?? ''));
    return undefined;
  },

  // Reading a `.mid` from the library into one generator (TASK-058/040T).
  //
  // ⛔ **Built on `generate_pattern` rather than as a second note fixture**, for
  // the reason `generate_song` gives one screen down: a spec that counts an
  // imported clip's notes and one that counts a generated clip's must not be
  // able to disagree. What this fixture is *for* is the routing — that the file
  // lands on the part it was dropped on, and that a file with nothing in it is
  // refused rather than opened.
  //
  // ⚠ The real command parses an SMF; `engine::smf_read` is tested in Rust and
  // this cannot re-test it. What a browser can show is that the page asked, with
  // the right part, and drew what came back.
  explorer_midi: (args?: InvokeArgs) => {
    const { path, part } = (args ?? {}) as { path?: unknown; part?: unknown };
    if (typeof path !== 'string' || !/\.midi?$/i.test(path)) {
      throw new Error('that is not a MIDI file in your sample library');
    }
    // A fixture for the empty case, so the refusal has something to refuse.
    if (/empty/i.test(path)) {
      return { ...(handlers.generate_pattern({ request: { part } }) as Pattern), lanes: [] };
    }
    return handlers.generate_pattern({ request: { part } });
  },

  // Separating a layered `.mid` (TASK-058D). ⚠ A fixture with three voices,
  // because a one-part answer could not tell a working split from a split that
  // silently returned only what it found first.
  explorer_midi_split: (args?: InvokeArgs) => {
    const path = String((args as { path?: unknown } | undefined)?.path ?? '');
    if (!/\.midi?$/i.test(path)) {
      throw new Error('that is not a MIDI file in your sample library');
    }
    // Opening a `.mid` is opening a file. See `noteRecent`.
    noteRecent(path);
    const of = (part: Part) => handlers.generate_pattern({ request: { part } }) as Pattern;
    const notes = (pattern: Pattern) =>
      pattern.lanes.reduce((sum, lane) => sum + lane.notes.length, 0);
    return [
      { part: 'bass', pattern: of('bass'), reason: 'lowestVoice', notes: notes(of('bass')) },
      {
        part: 'counter',
        pattern: of('counter'),
        reason: 'innerVoice',
        notes: notes(of('counter')),
      },
      {
        part: 'melody',
        pattern: of('melody'),
        reason: 'highestVoice',
        notes: notes(of('melody')),
      },
    ];
  },

  // ⛔ **Hearing a `.mid` (TASK-160).** In the plugin this renders the file into
  // the audition voice; a browser has neither, so what the mock can honestly do
  // is answer the shape — a length and whether the file was cut — and let the
  // panel's transport be exercised against it. ⚠ It does **not** pretend the
  // position advances: `preview_position` still answers a still playhead, which
  // is what a shell with no audio thread truthfully has.
  explorer_midi_audition: (args?: InvokeArgs) => {
    const path = String((args as { path?: unknown } | undefined)?.path ?? '');
    if (!/\.midi?$/i.test(path)) {
      throw new Error('that is not a MIDI file in your sample library');
    }
    return { seconds: 4, clipped: false };
  },

  // A whole `.mid` as an arrangement, for the Song tab (TASK-058D).
  //
  // ⛔ **Built on `generate_song` rather than a second arrangement fixture**, for
  // the reason `generate_song` itself gives: a spec that counts an imported
  // song's sections and one that counts a generated song's must not be able to
  // disagree. ⚠ What differs is what an *import* honestly is — no artist and no
  // seed, because a file carries neither.
  explorer_song: (args?: InvokeArgs) => {
    const path = String((args as { path?: unknown } | undefined)?.path ?? '');
    if (!/\.midi?$/i.test(path)) {
      throw new Error('that is not a MIDI file in your sample library');
    }
    const song = handlers.generate_song({ request: {} }) as Song;
    return { ...song, artistId: '', seed: '0' };
  },

  explorer_pick: () => undefined,
  explorer_remove: (args?: InvokeArgs) => {
    const path = String((args as { path?: unknown } | undefined)?.path ?? '');
    const at = libraryRoots.findIndex((root) => root.path === path);
    if (at >= 0) libraryRoots.splice(at, 1);
    return undefined;
  },
  explorer_open: () => undefined,
  // One folder's rows, for a node the producer expanded (TASK-058). ⚠ The
  // fixture is a shallow tree rather than one folder, because the defect the
  // tree was built for — *"you cannot go into those subfolders"* — only shows up
  // below the first level.
  explorer_list: (args?: InvokeArgs) => {
    const path = String((args as { path?: unknown } | undefined)?.path ?? '');
    const entries = libraryRows[path];
    // ⛔ Refuses an unknown folder rather than answering an empty one, because
    // the real command refuses anything outside the library — and an empty list
    // would let a containment bug read as "that folder happens to be empty".
    if (entries === undefined) {
      throw new Error('that is not a folder in your sample library');
    }
    return { roots: [], folder: path, parent: null, entries, truncated: false, picking: false };
  },
  // ⛔ **Refuses a folder and refuses an empty path**, because the real command
  // does: a mock that accepted anything would let the panel start offering a
  // folder as a draggable row without a single spec noticing.
  explorer_drop: (args?: InvokeArgs) => {
    const { lane, path } = (args ?? {}) as { lane?: unknown; path?: unknown };
    if (typeof lane !== 'string' || typeof path !== 'string' || path === '') {
      throw new Error('that is not a lane');
    }
    droppedSamples.set(lane, path);
    return undefined;
  },
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
  preview_load: (args?: InvokeArgs) => {
    noteRecent(String((args as { path?: unknown } | undefined)?.path ?? ''));
    return undefined;
  },
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

  // The pattern library (TASK-045A). ⛔ **This one really does store**, unlike
  // the preset mock above, and the difference is what the panel is for: saving
  // a pattern and finding it in the list is the whole gesture, so a fixture
  // that reported success without keeping it would make the feature look
  // broken in `vite dev` and untestable in Playwright. It lives for the
  // lifetime of the page, which is exactly as long as a browser has anywhere
  // to put it.
  patterns_list: () => [...savedPatterns.values()].sort((a, b) => b.savedAt - a.savedAt),
  pattern_save: (args?: InvokeArgs) => {
    const { name, savedAt, pattern } = (args ?? {}) as {
      name?: string;
      savedAt?: number;
      pattern?: Pattern;
    };
    if (pattern === undefined) throw new Error('that is not a pattern');
    const id = String(name ?? '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '');
    const summary = {
      id: id === '' ? 'pattern' : id,
      name: String(name ?? ''),
      artistId: pattern.artistId,
      part: pattern.part,
      bars: pattern.bars,
      bpm: pattern.bpm,
      savedAt: savedAt ?? 0,
      density: mockDensity(pattern),
    };
    savedPatterns.set(summary.id, summary);
    savedClips.set(summary.id, pattern);
    return summary;
  },
  pattern_load: (args?: InvokeArgs) => {
    const id = String((args as { id?: unknown } | undefined)?.id ?? '');
    const clip = savedClips.get(id);
    if (clip === undefined) throw new Error(`no pattern \`${id}\``);
    return clip;
  },
  pattern_delete: (args?: InvokeArgs) => {
    const id = String((args as { id?: unknown } | undefined)?.id ?? '');
    savedPatterns.delete(id);
    savedClips.delete(id);
    return undefined;
  },
};

/** The mock library, for the lifetime of the page. */
const savedPatterns = new Map<string, PatternSummaryLike>();
const savedClips = new Map<string, Pattern>();

type PatternSummaryLike = {
  id: string;
  name: string;
  artistId: string;
  part: string;
  bars: number;
  bpm: number;
  savedAt: number;
  density: number[];
};

/** The same histogram `plugin/src/patterns.rs` computes, in 32 columns. */
function mockDensity(pattern: Pattern): number[] {
  const columns = 32;
  const total = Math.max(
    1,
    pattern.bars * pattern.timeSigNum * ((pattern.ppq * 4) / pattern.timeSigDen),
  );
  const counts = new Array<number>(columns).fill(0);
  for (const track of pattern.lanes) {
    for (const note of track.notes) {
      const column = Math.min(columns - 1, Math.floor((note.startTick / total) * columns));
      counts[column] += 1;
    }
  }
  const busiest = Math.max(...counts);
  return busiest <= 0 ? counts : counts.map((count) => count / busiest);
}

/**
 * What the page has asked to copy, for a spec to assert the negative.
 *
 * ⛔ A gate is only tested by checking that nothing happened when it was shut.
 * `window.__freallyCopiedSamples` is how Playwright can see that.
 */
declare global {
  interface Window {
    __freallyCopiedSamples?: string[];
    /** Paths the page asked to reveal in the OS file manager (TASK-058C). */
    __freallyRevealed?: string[];
  }
}

export async function mockInvoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  if (typeof window !== 'undefined') {
    window.__freallyCopiedSamples = copiedSamples;
    // ⛔ Same reasoning: a browser cannot open Explorer, so the only thing a spec
    // can check is that the page asked for the right path.
    window.__freallyRevealed = revealed;
  }
  const handler = handlers[command];
  if (!handler) {
    throw new Error(
      `ipc-mock has no handler for "${command}". Add one in src/lib/ipc-mock.ts — ` +
        `silently returning undefined would hide the bug this test exists to catch.`,
    );
  }
  return handler(args) as T;
}
