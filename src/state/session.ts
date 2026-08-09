import { create } from 'zustand';

import { invoke } from '../lib/ipc';
import { isPlugin } from '../lib/ipc-plugin';
import { loadRoster } from '../lib/roster';
import type {
  DatasetProblem,
  Part,
  Pattern,
  RosterEntry,
  Scale,
  SessionDefaults,
  Song,
} from '../lib/ipc-types';
import { patternTicks } from '../components/PianoRoll/notes';
import { useHistory, type Snapshot } from './history';
import { entryFor, useVariations, type Variation } from './variations';
import { useUi, type GeneratorTab } from './ui';

/**
 * The one loop the product is about: pick someone, generate, hear it, and have
 * the notes land on the host's track (PRD § 1, US-001).
 *
 * One store rather than three, because these things are not independent:
 * generating replaces the pattern, which is what the sampler plays and what the
 * grid draws, and selecting someone else invalidates all of it. Splitting them
 * would mean keeping three stores in step by hand.
 */

/** What `host_session` reports from the DAW the plugin is loaded in. */
type HostSessionInfo = {
  tempo: number | null;
  timeSigNum: number;
  timeSigDen: number;
  playing: boolean;
};

/**
 * Bar counts the UI offers. Four is the default a pattern is demonstrated at.
 *
 * ⛔ **Two was removed on Mike's instruction, 2026-08-06:** *"bars for the
 * generators should be able to be 4 or 8 only, not 2"*, and then, clarifying
 * what replaces it: *"every new generation should generate 4/8 bars only and
 * you should be able to see all 8 bars."* A two-bar loop is a *view* of a
 * pattern rather than a length worth generating at — there is not enough room
 * in two bars for the fills and turnarounds the models author, so it made every
 * artist sound the same.
 *
 * ⚠ **A project saved at two bars still opens.** Nothing here validates a
 * restored value: `bridge.rs` clamps `bars` to `1..=MAX_BARS` on every path
 * into the engine, so a stored 2 generates a two-bar pattern exactly as before
 * — it simply is not offered as a new choice. Refusing it instead would break
 * sessions somebody already has.
 */
export const BAR_CHOICES = [4, 8] as const;

/**
 * Snapshot fields that are **not** read out of this store.
 *
 * ⛔ Each one is a whole document rather than a session value, and each is sent
 * conditionally by `send()` — `pattern` only when the clip was edited, `song`
 * only when the arrangement was. `SAVED_FIELDS` drives a `state[key]` lookup, so
 * naming them there would read `undefined` off the session store and save
 * nothing while claiming to.
 *
 * `songEdited` rides with `song` because it is the flag that decides whether the
 * document is worth storing at all; it has no meaning apart from it.
 */
type DocumentFields = 'patterns' | 'song' | 'songEdited';

/**
 * `SAVED_FIELDS` must name exactly the undo snapshot's fields except the
 * documents above.
 *
 * ⛔ A compile-time check rather than a comment. `snapshotOf` lists the fields
 * explicitly so the compiler catches one added to `Snapshot` and forgotten
 * there; this catches the other direction — a field added to `Snapshot` and to
 * `snapshotOf` but not to `SAVED_FIELDS`, which would save less than it undoes.
 */
type SavedFieldsCoverSnapshot =
  Exclude<keyof Snapshot, DocumentFields> extends (typeof SAVED_FIELDS)[number] ? true : never;
const SAVED_FIELDS_MATCH_SNAPSHOT: SavedFieldsCoverSnapshot = true;
void SAVED_FIELDS_MATCH_SNAPSHOT;

/**
 * The fields that are saved with the project — the single list.
 *
 * ⛔ **This exists because the same list was written out three times and
 * drifted three times.** `send()` builds the payload from it, the save
 * subscriber compares it, and the undo snapshot carries it plus `pattern`; when
 * those were three hand-maintained lists, `autoSync` shipped in one of them,
 * then `mood` and `audioEnabled` in two, then `mutedLanes` in one — each time
 * with the same symptom, where a direct change saved and an undone one did not.
 * Adding a persisted field is now one edit here.
 */
export const SAVED_FIELDS = [
  'selectedId',
  'seed',
  // ⛔ The record, saved beside the take. Without it a reopened project — or an
  // undo across an artist change — restored the five clips and lost the plan
  // they were written against, so the next Generate started a new record and
  // that part no longer belonged with the rest. TASK-141 behaviour surviving
  // exactly until the first reload is not the feature.
  'songSeed',
  'seedPinned',
  'bars',
  'pins',
  'autoSync',
  'mood',
  'audioEnabled',
  'mutedLanes',
  'soloedLanes',
  'lockedLanes',
  'edited',
  'editedParts',
] as const;

/**
 * The session values a user may pin, in the shape the engine's
 * `SessionOverrides` reads (FR-002).
 *
 * `null` means "not pinned", not "zero" — an absent override lets the artist's
 * own value stand, and sending a default in its place is how an artist's tempo
 * silently becomes whatever the UI happened to initialise. The seed box works
 * the same way, and for the same reason.
 */
export type SessionPins = {
  bpm: number | null;
  keyRoot: number | null;
  scale: Scale | null;
  swing: number | null;
  /**
   * The meter the producer set for this clip (TASK-041E).
   *
   * ⛔ Null means "whatever the host is in", not 4/4. Inside a DAW the meter
   * comes from the project, and pinning a default here would drag a 6/8 session
   * back to common time on the next Generate — see `host.rs::session_for`.
   */
  timeSigNum: number | null;
  timeSigDen: number | null;
};

export const NO_PINS: SessionPins = {
  bpm: null,
  keyRoot: null,
  scale: null,
  swing: null,
  timeSigNum: null,
  timeSigDen: null,
};

/** Has the user pinned anything at all? */
export function hasPins(pins: SessionPins): boolean {
  return Object.values(pins).some((value) => value !== null);
}

/**
 * The five generators' clips, keyed by part (TASK-119).
 *
 * `Partial` rather than a full record with nulls: a part that has never been
 * generated is *absent*, and a part that generated silence is present with
 * empty lanes. Collapsing those two into `null` is what would make an 808-is-
 * the-bass style look like a bass tab nobody had pressed Generate on.
 */
export type PatternsByPart = Partial<Record<Part, Pattern>>;

/**
 * The five parts, in the order Generate-all fills them.
 *
 * ⛔ **Drums first, and the order is the dependency rather than the tab strip.**
 * `parts.rs` states it: harmony, then the melody written against it, then the
 * counter answering the melody, then the bass locking to the kick. The engine
 * regenerates every dependency per request, so this order does not change what
 * comes back — but it is the order a producer watches fill in, and showing a
 * countermelody land before the melody it answers reads as a bug.
 */
/** `list` with `part` added, kept sorted so equal sets compare equal. */
function withEdit(list: Part[], part: Part): Part[] {
  return list.includes(part) ? list : [...list, part].sort();
}

/** `list` without `part`. Returns the same array when nothing changed, so an
 * action that edits nothing does not push an undo entry. */
function withoutEdit(list: Part[], part: Part): Part[] {
  return list.includes(part) ? list.filter((held) => held !== part) : list;
}

export const GENERATED_PARTS: readonly Part[] = [
  'drums',
  'chords',
  'melody',
  'counter',
  'bass',
];

/**
 * Which part a generator tab edits, or `null` for a tab that is not a part.
 *
 * ⛔ Song is not a part and never becomes one — it is an *arrangement* of the
 * five, and it lives in `useSong`. The `null` is what keeps it out of the
 * per-part slots.
 *
 * Lives here rather than in `CenterStage` because the arm path needs it too,
 * and a second copy beside the first is the drift this codebase has been bitten
 * by repeatedly.
 */
export const TAB_PART: Record<GeneratorTab, Part | null> = {
  drums: 'drums',
  melody: 'melody',
  counter: 'counter',
  bass: 'bass',
  chords: 'chords',
  song: null,
};

/** The clip the tab now showing would draw, or `null` if it has none. */
export function patternForTab(state: SessionState, tab: GeneratorTab): Pattern | null {
  const part = TAB_PART[tab];
  return part === null ? null : (state.patterns[part] ?? null);
}

/**
 * The clips that would actually be armed right now (TASK-127).
 *
 * ⛔ **One predicate, because the transport was answering an older question.**
 * `armCurrentPattern` arms *every generated part that is switched on, whatever
 * tab you are looking at*, while `TransportBar` still gated Play and Stop on
 * `useActivePattern() !== null` — the pre-TASK-127 rule. The two disagreed in
 * both directions:
 *
 * - Generate drums only, then click the Melody tab: drums are armed and would
 *   sound, and Play is **dark** because the melody slot is empty.
 * - Switch every part off on a generated tab: `armCurrentPattern` disarms, and
 *   Play stays **lit** — pressing it sets `running` over an empty schedule, so
 *   the UI reports playing forever with a marker that never moves. The bridge
 *   already refuses this (`toggling_every_generator_off_is_refused_rather_than
 *   _arming_silence`); the button simply had not been told.
 *
 * ⚠ Empty on the Song tab by design: `TAB_PART.song` is `null` because the
 * arrangement is not a part, and `SongTimeline` arms it itself.
 */
/**
 * Can the transport be driven right now?
 *
 * ⛔ **One predicate, exported, because it was being restated.** The Play
 * button and the Space handler each carried their own half of it: Space checked
 * only `canDriveTransport()`, so pressing it with nothing generated set
 * `running` over an empty schedule — the app then reported playing forever,
 * Play rendered as a disabled Pause, Stop was disabled with it, and only a
 * second Space got out.
 *
 * ⚠ The Song tab asks a different question: the arrangement is not a part, so
 * there is nothing in `armedClips` to count and the song itself is the answer.
 * Reading `song !== null` on *every* tab lit Play on a part tab whose transport
 * `armCurrentPattern` had just disarmed.
 */
export function canDrive(
  patterns: PatternsByPart,
  partsOff: readonly Part[],
  tab: GeneratorTab,
  song: unknown,
): boolean {
  return tab === 'song' ? song !== null : armedClips(patterns, partsOff, tab).length > 0;
}

export function armedClips(
  patterns: PatternsByPart,
  partsOff: readonly Part[],
  tab: GeneratorTab,
): Pattern[] {
  if (TAB_PART[tab] === null) return [];
  return GENERATED_PARTS.filter((part) => !partsOff.includes(part))
    .map((part) => patterns[part])
    .filter((clip): clip is Pattern => clip !== undefined && clip !== null);
}

/**
 * The clip on screen — the active tab's, or `null` on Song and on a part that
 * has not been generated.
 *
 * ⛔ **This is what "the pattern" used to mean, and the two stopped being the
 * same thing at TASK-119.** Before it there was one slot, so "the pattern in the
 * store" and "the pattern you are looking at" were identical; now the store
 * holds five and only one is on screen. Everything that used to read
 * `session.pattern` for a readout — the transport position, the session chips,
 * the editor — wants this rather than any particular slot.
 */
export function useActivePattern(): Pattern | null {
  const activeTab = useUi((s) => s.activeTab);
  return useSession((s) => patternForTab(s, activeTab));
}

type SessionState = {
  roster: RosterEntry[];
  problems: DatasetProblem[];
  rosterLoaded: boolean;

  selectedId: string | null;
  /**
   * One clip per part — the five generators each keep their own (TASK-119).
   *
   * ⛔ **This was a single `pattern` until 2026-08-04, and the difference is the
   * whole defect.** With one slot, generating a bassline replaced the melody
   * that was there, and `CenterStage` covered for it by drawing a tab's editor
   * only when the slot happened to hold that tab's part — so the melody did not
   * look corrupted, it looked *absent*. Mike found it in FL Studio; no gate
   * could, because "the other four are still there" was asserted nowhere.
   *
   * ⚠ **Absent and empty are different.** A missing key is "never generated",
   * which is the tab's ready-to-generate state. A present pattern with no notes
   * is a real answer — a style whose 808 *is* the bassline authors no separate
   * bass part, and `parts.rs` returns empty lanes for it on purpose.
   *
   * ⚠ Parts only agree with each other when they were generated from the same
   * seed (`engine/src/parts.rs`). `generate` reuses the session seed for exactly
   * that reason; see the comment there before changing it.
   */
  patterns: PatternsByPart;
  /**
   * Which parts are hand-edits rather than the seed's own output.
   *
   * ⛔ **`edited` was one flag for the whole session, and once there were five
   * slots that flag became a defect.** `generate(part)` cleared it, and `send()`
   * uses it to decide whether *any* clip is saved — so generating a bassline
   * wrote the project with no clips at all and silently deleted a melody the
   * producer had spent an hour drawing. The flag has to be per-part because the
   * thing it describes is per-part.
   *
   * ⚠ `edited` is kept beside this and is exactly `editedParts.length > 0`. It
   * stays because it is what crosses the bridge and what `put()` trusts on
   * restore; this is the page's own finer-grained record of the same fact.
   */
  editedParts: Part[];
  bars: number;
  /**
   * The seed to generate with, as typed. A string because a u64 does not
   * survive a JSON number, which is the same reason `Pattern.seed` is one.
   *
   * ⚠ **This box holds two different things and [`seedPinned`] is what tells
   * them apart** — a seed the producer chose, and the seed the engine last
   * picked and echoed back for them to read. Never decide from `seed` alone.
   */
  seed: string;
  /**
   * The **record's** seed — what the five parts agree on (TASK-141).
   *
   * ⛔ **This is what buys back what the Defect 2 fix gave up.** `seed` above is
   * the *take*, and it rerolls on every press by design. But `engine/parts.rs`
   * only guarantees the five parts belong together when they share a harmonic
   * plan — so before this existed, the ordinary workflow (Generate on Drums,
   * switch tab, Generate on Melody) drew two unrelated seeds and wrote the
   * melody against a progression the chords tab had never seen.
   *
   * Carried back on every Generate, so the take varies and the record does not.
   * Empty means "the next Generate starts a new record", which is what a fresh
   * session and a newly selected artist both mean.
   *
   * ⚠ Deliberately **not** pinned by the seed lock. The lock is about the
   * producer's typed take; the record is carried automatically because a
   * producer should not have to know this exists to get coherent parts.
   */
  songSeed: string;
  /**
   * Whether [`seed`] is the producer's choice rather than the engine's echo.
   *
   * ⛔⛔ **The whole of the defect Mike found in Ableton on 2026-08-06:**
   * *"when i clicked to generate seeds for Drake and did it over and over
   * again, the seed stayed the same and there was no variation"*. `generate`
   * echoes the seed the engine used back into the box — which it must, see
   * below — and the old code then re-sent it on the next press. So the first
   * Generate rolled a seed and **every press after that regenerated it**,
   * making the product look like it held one beat per artist.
   *
   * ⛔ **Deleting the echo is not the fix, and that was the tempting one.**
   * `engine/src/parts.rs` guarantees the five parts agree *only* when they
   * share a seed, and US-004 ("paste a seed, get the same beat") needs a typed
   * seed to survive. Both of those are the echo. What was missing is the
   * distinction the session already draws for tempo, key and scale: a value the
   * *user* pinned is reused, a value the *engine* chose is displayed.
   *
   * So: pinned means every Generate reuses it; unpinned means every Generate
   * asks for a fresh one and the box shows what came back, to read and to copy.
   * Typing pins, clearing unpins, and the lock in `SeedChip` says which it is —
   * a box that silently pinned itself after the first Generate is what caused
   * this, so the mode is never left to be inferred.
   */
  seedPinned: boolean;

  generating: boolean;
  /** What went wrong last, for the user rather than the console. */
  error: string | null;

  playing: boolean;
  /** Position through the loop, 0–1, from the audio thread at 30 Hz. */
  playhead: number;
  /**
   * Why the transport cannot be driven from here, if it cannot.
   *
   * A human-readable reason for the Play button's tooltip — never a decision on
   * its own. [`canDriveTransport`] is what decides.
   */
  playbackFailure: string | null;
  /**
   * Whether this UI is running without a host (TASK-041T).
   *
   * ⛔ Decides who owns Play and Pause, and it is not a cosmetic difference.
   * In a DAW the project's transport is the transport, and a Play button of
   * ours would be a second one that cannot move the first. In the standalone
   * there is no host, so these are the only transport controls there are.
   *
   * Arrives with `playbackFailure` from one `playback_status` reply rather than
   * from a command of its own: two commands answering from the same source is
   * two flags that can drift, and the UI would recombine them into one decision
   * anyway.
   */
  standalone: boolean;

  /** What the user pinned. Everything absent is the artist's to choose. */
  pins: SessionPins;
  /**
   * The tempo the DAW is running at, or `null` outside a host.
   *
   * Held separately from `defaults` because it answers a different question:
   * `defaults` is what the *artist* asks for, and this is what the *project*
   * is. When both exist the project wins — a clip generated at the artist's
   * 140 inside a 92 BPM song does not fit the song it was asked for — and the
   * chip says which one it is showing rather than leaving the user to guess.
   */
  hostTempo: number | null;
  /**
   * Whether the tempo follows the DAW (TASK-P15).
   *
   * ⛔ Not a duplicate of `pins.bpm === null`. There are *three* states, and
   * the pin only distinguishes two of them: pinned, following the host, and
   * using the artist's own tempo. Without this the artist's authored tempo is
   * unreachable inside a running project.
   */
  autoSync: boolean;
  /**
   * The pinned mood, or `null` for "Any" (TASK-040V).
   *
   * "Any" is not "no mood": the engine picks one from the seed, so a reroll can
   * land on a different kind of record by the same artist — which is the whole
   * point of modes. Pinning holds it to one. `pattern.mood` is what it landed
   * on, the same way the seed box echoes the seed it used.
   */
  mood: string | null;
  /**
   * Whether the plugin plays its own preview kit (FMM-S02).
   *
   * ⛔ Off is **MIDI-only, and a first-class mode rather than a degraded one**:
   * a producer routing these notes into Battery does not want the preview kit
   * doubling every hit. It is what the plugin did before it had a sampler.
   */
  audioEnabled: boolean;
  /**
   * Lanes whose audio is muted (FMM-S02).
   *
   * ⛔ **Audio only, and the distinction is the whole feature.** A muted lane
   * still goes out as MIDI — the notes are already on the host's track by the
   * time the preview renders — so this silences our kick without removing the
   * kick from the pattern anyone routed away. Muting a *part* is the host's
   * job, on the track the notes landed on.
   */
  mutedLanes: string[];
  /**
   * Lanes soloed in the preview (TASK-043).
   *
   * ⛔ **Its own list rather than a rewrite of `mutedLanes`**, because solo is
   * *'everything except these, for now'*. Folding it into the mutes would make
   * the routing the producer had chosen before soloing unrecoverable — clicking
   * S and clicking it again would silently rewrite it. The audio thread combines
   * the two into one mask; both sides of the bridge keep them apart.
   *
   * Empty means **no solo**, never 'solo nothing'.
   */
  soloedLanes: string[];
  /**
   * Lanes held across a reroll (TASK-044).
   *
   * ⛔ **A lock is about what the next Generate may touch, not about editing.**
   * Locking the kick and pressing Generate has to give back a new hat pattern
   * over *that* kick, byte for byte — which is why the splice happens after the
   * engine answers rather than as a mask sent to it. The engine is
   * deterministic, so keeping the lane the producer is looking at is exact by
   * construction and needs no cooperation from the generator.
   *
   * ⚠ **What it cannot do, said out loud:** a locked *kick* does not
   * retro-influence a rerolled bass. `bassline.rhythm = "mirror_kick"` reads
   * the kit the engine rebuilds from the seed, not the one on screen, so a bass
   * generated after a kick was locked mirrors the engine's kick. `Seeds::drums`
   * is the existing seam for that and is TASK-141's, not this one's.
   */
  lockedLanes: string[];
  /**
   * Whether the clip on screen is an edit rather than the seed's own output.
   *
   * ⛔ **This is what makes an edited clip survive closing the project.**
   * `plugin/src/state.rs` saves the *inputs* — artist, seed, pins — because the
   * engine is deterministic, so a few hundred bytes reopen the same pattern.
   * The moment a producer moves a note that stops being true, and regenerating
   * from the seed would reopen the session having quietly undone their editing.
   * From here on the clip itself is saved instead, and this is the flag that
   * says which of the two the project file should trust.
   */
  edited: boolean;
  /**
   * What the selected style asks for, read the moment it is selected.
   *
   * `null` before the first selection and whenever the read failed — the chips
   * then show nothing rather than a value from the artist before this one.
   */
  defaults: SessionDefaults | null;
  /**
   * The artist just switched to, while pins from the last one are still held.
   *
   * The switch has already happened; this only asks what to do with the pins
   * (PRD FR-002: "user overrides persist until artist change — keep or adopt").
   * Blocking the selection on the answer would make the prompt a toll gate on
   * browsing, which is the one thing the roster is for.
   */
  pendingArtist: RosterEntry | null;

  init: () => Promise<void>;
  /**
   * Re-read the roster alone, after the user saves or deletes a style.
   *
   * ⛔ Not `init()`, which also restores the session, asks who owns the
   * transport and reloads the drag capability — none of which changed because
   * somebody named a style. Running it again would rewind the producer's
   * session to whatever was last persisted, which is a spectacular thing for a
   * Save button to do.
   */
  refreshRoster: () => Promise<void>;
  select: (id: string) => void;
  /** Type or paste a seed. Anything non-empty pins it; clearing the box unpins. */
  setSeed: (seed: string) => void;
  /**
   * Hold the seed the engine just picked, or hand it back (the lock button).
   *
   * The other half of "typing pins it": a producer who likes what they just
   * generated wants *that* seed held without retyping the twenty digits already
   * in front of them.
   */
  setSeedPinned: (pinned: boolean) => void;
  setBars: (bars: number) => void;
  setPin: <K extends keyof SessionPins>(field: K, value: SessionPins[K]) => void;
  setAutoSync: (on: boolean) => void;
  /** Pin the mood, or hand it back to the seed with `null`. */
  setMood: (mood: string | null) => void;
  /** Let the plugin play its preview kit, or go MIDI-only. */
  setAudioEnabled: (on: boolean) => void;
  /** Silence one lane in the preview, or let it back in (FMM-S02). */
  setLaneMuted: (lane: string, muted: boolean) => void;
  /**
   * Hold one lane across the next Generate, or let it reroll (TASK-044).
   *
   * ⛔ **View state that changes what generation *keeps*, never what it
   * writes.** Nothing about a lock reaches the engine; the splice happens on
   * the answer.
   */
  setLaneLocked: (lane: string, locked: boolean) => void;
  /**
   * Solo one lane in the preview, or take it out of the solo set (TASK-043).
   *
   * ⛔ **View and playback state, never an edit.** Like the mutes, this changes
   * what the preview sampler sounds and nothing about the notes — what is
   * exported and what reaches the host is identical either way.
   */
  setLaneSolo: (lane: string, solo: boolean) => void;
  /**
   * Move the playhead, as a fraction of the pattern (TASK-041T).
   *
   * Click anywhere on the timeline and playback continues from there. In the
   * plugin the audio thread picks this up on its next block; the mock has no
   * audio thread and keeps the local move.
   */
  seek: (progress: number) => Promise<void>;
  /** Ask the host what tempo it is running at. No-op outside a plugin. */
  refreshHost: () => Promise<void>;
  /**
   * Replace the session with a preset's, and save the result.
   *
   * Unlike the project restore this has no "only if nothing is selected" guard:
   * loading a preset is a deliberate act, and refusing it because an artist was
   * already chosen would make the control do nothing most of the time.
   */
  applyPreset: (session: SavedSession) => void;
  /** Keep the pinned session over the new artist's defaults. */
  keepPins: () => void;
  /** Drop every pin and let the new artist decide. */
  adoptDefaults: () => void;
  /**
   * May this page drive the transport at all?
   *
   * ⛔ **The single predicate behind Play, Pause and their disabled state.**
   * It was briefly three derivations across two wire fields — a `disabled`
   * computed from the reason string, a conditional click handler, and the
   * bridge's own refusal — which could each stop agreeing with the others with
   * nothing failing loudly. Both terms are needed: `standalone` because a host
   * owns its own transport, and `playbackFailure` because the standalone can
   * still have a reason of its own (no output device, a kit that failed to
   * decode), and then Play must be disabled rather than merely unhelpful.
   */
  canDriveTransport: () => boolean;
  /**
   * Generate one part, defaulting to drums.
   *
   * The default keeps every existing caller — and the drums-only e2e path —
   * saying exactly what it said before, while the piano roll's tabs name the
   * part they are showing.
   */
  generate: (part?: Part) => Promise<void>;
  /**
   * Generate all five parts from one seed (TASK-120).
   *
   * ⛔ **One seed, and that is the whole point rather than an optimisation.**
   * `engine/src/parts.rs` guarantees the five agree only when they share one —
   * a melody is written against the harmony and around the drums, so five parts
   * drawn from five seeds are five individually-correct clips that were never
   * written against each other. The first reply fixes the seed for the rest,
   * which is what makes an empty seed box still produce a coherent record.
   */
  generateAll: () => Promise<void>;
  /** Empty one part's slot, leaving the other four (TASK-121). */
  clearPart: (part: Part) => void;
  /** Empty all five (TASK-121). */
  clearAll: () => void;
  /**
   * Replace the pattern with an edited one (TASK-041).
   *
   * ⛔ **Called once per completed gesture, never per pointermove.** The history
   * subscriber records every write, and `history.ts` lists `pattern` as
   * *discrete* so pattern entries deliberately never coalesce — that is right
   * for a generation, which is one deliberate act, and catastrophic for a drag,
   * which would land one undo step per frame. The live drag is held in
   * `state/editing.ts` as a delta and applied here on pointerup.
   *
   * ⛔ **This is the moment a clip stops being derived from its seed.** Until an
   * edit it is reproducible from `seed` alone, which is what keeps project files
   * tiny; afterwards the notes *are* the document and the seed is only where it
   * started. See `materialised` below and `plugin/src/state.rs`.
   */
  /**
   * Put one of a song's clips into the editors (TASK-071's drill-in).
   *
   * Its own action rather than a bare `set`, because two things have to happen
   * together and neither is optional: the clip becomes the session's, and the
   * tab moves to the part that can draw it. A caller doing one without the
   * other leaves a melody clip open on the drum grid.
   */
  openClip: (pattern: Pattern, part: Part) => void;
  editPattern: (next: Pattern) => void;
  /** Run our own transport. Standalone only — in a host this is the DAW's. */
  play: () => Promise<void>;
  /** Hold it where it is. Standalone only, for the same reason. */
  pause: () => Promise<void>;
  stop: () => Promise<void>;

  /**
   * Go back to a generation, whole (TASK-045).
   *
   * ⛔ **Artist, mood, seed, bars and pins together — never the seed alone.**
   * A recall that restored the number only would regenerate a *different* beat
   * whenever the artist had changed since, which is the readout lying about how
   * you got there.
   */
  recallVariation: (entry: Variation) => Promise<void>;
  /** Step back through the operation log (FMM-U01). No-op at the baseline. */
  undo: () => void;
  redo: () => void;
};

/**
 * The fields an undo step restores.
 *
 * Everything else in the store is either derived (`defaults`), reported by the
 * host (`hostTempo`), or transient (`generating`, `error`, the transport) — and
 * restoring any of those would undo something the user did not do.
 */
/**
 * Put the locked lanes back, exactly as they were (TASK-044).
 *
 * ⛔⛔ **This is the whole lock mechanism, and it is deliberately *not* a mask
 * sent to the engine.** Generation is a pure function of `(model, ctx, seeds)`,
 * so the take the producer is looking at cannot be preserved by asking the
 * generator to preserve it — it would have to be told which notes, which means
 * sending the notes, which is exactly what `Seeds` exists to avoid. Keeping the
 * track object the page already holds is byte-identical by construction and
 * needs no cooperation from anything.
 *
 * ⚠ **A lane locked before it existed is not invented.** If the previous
 * pattern has no track for it — the producer locked a lane the last take did
 * not write — the new one's is kept, because there is nothing to hold.
 *
 * ⚠ **And the limit, said out loud rather than discovered:** a locked *kick*
 * does not retro-influence a rerolled bass. `bassline.rhythm = "mirror_kick"`
 * reads the kit the engine rebuilds from the seed, not the one on screen, so
 * the bass mirrors the engine's kick and not the held one. `Seeds::drums` is
 * the seam that would fix it and belongs to TASK-141.
 */
function withLocks(next: Pattern, previous: Pattern | undefined, locked: string[]): Pattern {
  if (locked.length === 0 || previous === undefined) return next;
  // ⛔⛔ **Clipped to the *new* clip, and it was not.** The held track was
  // spliced in whole, so locking the kick on eight bars, dragging the bars chip
  // down to four and pressing Generate put an eight-bar kick inside a clip whose
  // `bars` says four. `toCells` and `columnDensity` both bounds-check, so the
  // grid drew a clean four bars — while the notes were still in `lanes` and went
  // to the host, to `to_midi` and to `stem_files`. The producer dragged out a
  // four-bar clip and got a kick stem playing in bars five to eight that nothing
  // on screen had ever shown them.
  //
  // ⚠ **The track object survives when nothing needed clipping**, which the
  // ten-reroll test asserts with `toBe`: keeping the reference is what makes a
  // held lane exact rather than merely equal, and it is what lets `generate`
  // tell "a lock landed" from "nothing changed" by identity.
  const end = patternTicks(next);
  const held = previous.lanes
    .filter((track) => locked.includes(track.lane))
    .map((track) => {
      const inside = track.notes.filter((note) => note.startTick < end);
      return inside.length === track.notes.length ? track : { ...track, notes: inside };
    });
  if (held.length === 0) return next;

  const byLane = new Map(held.map((track) => [track.lane, track]));
  const lanes = next.lanes.map((track) => byLane.get(track.lane) ?? track);
  // A locked lane the *new* take did not write still has to come back, or
  // rerolling into a sparser pattern would silently drop it.
  for (const track of held) {
    if (!lanes.some((existing) => existing.lane === track.lane)) lanes.push(track);
  }
  return { ...next, lanes };
}

/**
 * Turn `lane` on or off in one of the three lane lists, or answer `null` when
 * the list already says what the caller is asking for.
 *
 * ⛔ **Sorted, so each list is a set rather than a history of the order the
 * rows were clicked in.** Two projects that mute the same two lanes must save
 * the same bytes, or an undo entry and a project diff both record a change
 * nobody made. This was written out three times — once per list — and the
 * copies had already begun to drift in their variable names; the rule belongs
 * in one place, and each setter keeps only what is actually different about it,
 * which is *when it persists*.
 */
function toggledLanes(current: string[], lane: string, on: boolean): string[] | null {
  if (current.includes(lane) === on) return null;
  return on ? [...current, lane].sort() : current.filter((held) => held !== lane);
}

function snapshotOf(state: SessionState): Snapshot {
  // ⛔ `SAVED_FIELDS` plus `pattern`. The undo stack and the saved session
  // carry the same fields for the same reason — an undone change that never
  // reaches the project reopens contradicting what the UI just showed — and
  // `pattern` is the one addition, because it is restored rather than saved
  // (the engine regenerates it from the seed).
  // ⛔ Written out rather than built from `SAVED_FIELDS` by `Object.fromEntries`.
  // That form needed an `as Omit<Snapshot, 'pattern'>` cast, and the cast is
  // exactly what would stop the compiler noticing a field added to `Snapshot`
  // and forgotten here — the drift this whole arrangement exists to prevent.
  // `SAVED_FIELDS_MATCH_SNAPSHOT` below keeps the two lists honest instead.
  const {
    selectedId,
    seed,
    songSeed,
    seedPinned,
    bars,
    pins,
    autoSync,
    mood,
    audioEnabled,
    mutedLanes,
    soloedLanes,
    lockedLanes,
    edited,
  } = state;
  // The arrangement lives in its own store and is read through the seam, for
  // the same reason `send()` reads it there — see `registerSongDocument`.
  //
  // ⛔ **Only when it belongs to the artist that is selected, and this is not
  // belt-and-braces — it is the fix for a real ordering bug.** `song.ts` imports
  // `session.ts`, so this store's subscribers are registered first and run first
  // for the same `set`. When `select()` writes `selectedId`, the history
  // recorder therefore runs *before* `song.ts` clears the arrangement — and
  // filed the outgoing artist's song under the incoming artist's id. Ctrl+Z then
  // brought the previous artist's whole record back under a different name, and
  // the next save wrote it to the project. `put()` did the same thing to a
  // preset load.
  //
  // Comparing the ids rather than reordering the subscribers is what makes it
  // robust: a `Song` knows which artist it was built for, so this cannot be
  // broken again by a change to registration order.
  const held = readSongDocument();
  const arrangement =
    held.song !== null && held.song.artistId !== state.selectedId
      ? { song: null, edited: false }
      : held;
  return {
    selectedId,
    seed,
    songSeed,
    seedPinned,
    bars,
    pins,
    autoSync,
    mood,
    audioEnabled,
    mutedLanes,
    soloedLanes,
    lockedLanes,
    edited,
    editedParts: state.editedParts,
    patterns: state.patterns,
    song: arrangement.song,
    songEdited: arrangement.edited,
  };
}

/**
 * True while undo or redo is writing, so the subscriber below does not record
 * the restore as a fresh edit — which would push a new entry on every undo and
 * make the stack impossible to walk back out of.
 *
 * A module flag rather than store state: zustand calls subscribers synchronously
 * inside `set`, so it is only ever true for the duration of one call.
 */
let applying = false;

/**
 * True while [`recallVariation`](SessionState) is regenerating.
 *
 * ⛔⛔ **A recall is not a generation, and without this the history logged
 * itself.** Stepping back regenerates the take — that is what makes an entry
 * tens of bytes instead of a clip — and `generate` records every pattern it
 * lands. So walking back three takes appended three more, the counter climbed
 * while the producer moved *backwards*, and the log stopped being a record of
 * what they pressed. The same shape and the same reason as `applying` above:
 * zustand calls subscribers synchronously inside `set`, so this is only ever
 * true for the duration of one call.
 */
let recalling = false;

function applySnapshot(
  snapshot: Snapshot,
  set: (partial: Partial<SessionState>) => void,
  get: () => SessionState,
): void {
  const from = get().selectedId;

  // ⛔ The documents are peeled off rather than passed through. `set` merges
  // whatever it is handed, so writing the whole snapshot would put `song` and
  // `songEdited` into the *session* store — a second copy of the arrangement,
  // living beside the real one in `useSong` and drifting from it the moment
  // either changed.
  const { song, songEdited, ...session } = snapshot;

  applying = true;
  try {
    set(session);
    // Inside the guard, because `applySongDocument` writes to a store the song
    // module records edits from. Outside it, stepping back one arrangement edit
    // would immediately record the restored state as a fresh one and undo would
    // never move past it.
    applySongDocument({ song, edited: songEdited });
  } finally {
    applying = false;
  }

  // ⛔ **The restore has to reach the project file, and no subscriber does it
  // for an arrangement-only step.** The `SAVED_FIELDS` subscriber returns early
  // when no saved session field changed and the pattern subscriber returns
  // early when the clip is unchanged — an undo of a resize changes neither, and
  // `applySongDocument` writes `useSong` directly, bypassing
  // `noteDocumentChange` (which the `applying` flag would have blocked anyway).
  // So the producer undid a section resize, watched it snap back, closed the
  // project and reopened it to find the resize still there. That is word for
  // word the failure this file's own persist comment says drove the single
  // `SAVED_FIELDS` list into existence.
  //
  // Unconditional rather than gated on the song having changed: `persist` is
  // debounced and idempotent, and the alternative is a fourth place that has to
  // decide what counts as a change.
  persist();

  // `defaults` belongs to whichever artist was selected when it was read, so
  // stepping across an artist change has to re-read it — otherwise the chips
  // keep showing the previous artist's tempo under the restored one's name,
  // which is the readout-that-lies failure `loadDefaults` already guards.
  //
  // `pendingArtist` is cleared rather than restored: the keep-or-adopt prompt
  // asks about a switch the user just made, and an undo is not that switch.
  if (snapshot.selectedId !== from) {
    set({ defaults: null, pendingArtist: null });
    if (snapshot.selectedId !== null) void loadDefaults(snapshot.selectedId, set, get);
  }
}

/**
 * The drums' take seed, but only while it still names the drums on screen.
 *
 * ⛔⛔ **A seed alone does not identify a pattern — `(model, ctx, seed)` does.**
 * `Part::Bass` rebuilds its reference kit by re-running `drums::generate` at
 * this seed, which reproduces the clip the producer is looking at *only* while
 * the session it was built in still applies. Change the bars chip, pin a tempo,
 * pick a different mood, or start a new record between the two Generates and the
 * rebuilt kick lands on different ticks — so a `mirror_kick` bass mirrors kicks
 * nobody is playing, which is the narrower form of the defect `drumsSeed` was
 * added to close.
 *
 * ⚠ **`null` is a real answer, not a failure.** It falls back to the record's
 * own canonical kit, which is coherent — the melodic parts use it deliberately —
 * rather than confidently wrong.
 *
 * ⚠ Only the fields that reach `SessionContext` are compared. `key_root` and
 * `scale` are sampled from the *song* seed, so `songSeed` covers them; `mood`
 * picks the mode, which can retune the whole session block.
 */
export function mirrorableDrumsSeed(
  drums: Pattern | undefined,
  now: { bars: number; songSeed: string; pins: SessionPins; mood: string | null },
): string | null {
  if (!drums) return null;
  if (drums.bars !== now.bars) return null;
  // A fresh record means the drums belong to a different song entirely.
  if (now.songSeed !== '' && drums.songSeed !== now.songSeed) return null;
  // ⚠ Normalised: `mood` is `skip_serializing_if = "Option::is_none"` on the
  // Rust side, so a pattern generated without one arrives with the field
  // *absent* rather than null — and `undefined !== null` would withhold the seed
  // on the commonest case there is, silently giving up the fix.
  if ((drums.mood ?? null) !== now.mood) return null;
  // A pinned tempo or meter moves the grid the kick is written on.
  if (now.pins.bpm !== null && drums.bpm !== now.pins.bpm) return null;
  if (now.pins.timeSigNum !== null && drums.timeSigNum !== now.pins.timeSigNum) return null;
  if (now.pins.timeSigDen !== null && drums.timeSigDen !== now.pins.timeSigDen) return null;
  return drums.seed;
}

/** The message an IPC rejection carries, without leaking `[object Object]`. */
export function reason(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

/**
 * Read what a style asks for, for the chips.
 *
 * A failure here is deliberately not an `error`: the banner sits under the
 * Generate button and says a generation went wrong, and a readout that could
 * not be filled in is not that. The chips fall back to showing nothing, which
 * is what they showed before the artist was picked.
 *
 * The id is re-checked before the state is written, because clicking through a
 * roster starts a read per artist and they do not have to come back in order —
 * the last one to *arrive* would otherwise win over the one selected.
 */
async function loadDefaults(
  id: string,
  set: (partial: Partial<SessionState>) => void,
  get: () => SessionState,
): Promise<void> {
  try {
    const defaults = await invoke<SessionDefaults>('session_defaults', { styleId: id });
    if (get().selectedId === id) set({ defaults });
  } catch {
    if (get().selectedId === id) set({ defaults: null });
  }
}

/**
 * The session as the *plugin* stores it, which the host writes into the
 * project file (TASK-P07). Field-for-field what `PluginSession` in
 * `plugin/src/state.rs` reads and writes.
 *
 * `windowSize` is deliberately absent: the editor owns it, and the plugin
 * carries the stored value over any write that does not mention it.
 */
export type SavedSession = {
  selectedId: string | null;
  seed: string;
  /**
   * The record every part was written against (TASK-141).
   *
   * ⚠ **Optional, because every project written before this exists without
   * it** — and absent means 'start a new record on the next Generate', which
   * is exactly the pre-TASK-141 behaviour those projects already had.
   */
  songSeed?: string;
  /**
   * Whether `seed` was the producer's choice rather than the engine's echo.
   *
   * ⚠ **Absent means "this project predates the distinction", and it is read as
   * pinned** — see `put()`. Every project written before the seed could be
   * unpinned reused its stored seed on every Generate, so that is what those
   * projects reopen doing.
   */
  seedPinned?: boolean | null;
  bars: number | null;
  pins: Partial<SessionPins> | null;
  /**
   * The clips as edited, when the seed no longer describes them (TASK-041).
   *
   * Absent for every session nobody has drawn in — see `edited` on the store,
   * and `PluginSession::patterns` on the other side of the bridge.
   *
   * ⚠ A map since TASK-119. `pattern` is still read for projects written before
   * that, because a producer's saved work must survive the change that gave the
   * other four parts somewhere to live.
   */
  patterns?: PatternsByPart | null;
  /** @deprecated Pre-TASK-119 projects: one clip, whichever part it was. */
  pattern?: Pattern | null;
  edited?: boolean;
  /**
   * Whether the tempo follows the host (TASK-P15).
   *
   * Optional on the way in because a project saved before it existed does not
   * carry it, and absent must mean **on** — the plugin's own
   * `auto_sync_default` makes the same choice for the same reason.
   */
  autoSync?: boolean;
  /**
   * The pinned mood, absent for "Any" (TASK-040V).
   *
   * ⛔ Only a pin is stored. "Any" means the mood is picked from the seed, so
   * the same seed reopens on the same mood with nothing saved — the same
   * argument that lets the pattern itself go unsaved.
   */
  mood?: string | null;
  /**
   * Whether the preview sampler sounds (FMM-S02).
   *
   * Optional on the way in, and absent means **on** — the plugin's own
   * `audio_enabled_default` makes the same choice for the same reason.
   */
  audioEnabled?: boolean;
  /**
   * Lanes whose audio is muted (FMM-S02).
   *
   * ⛔ Sent on every save, even when empty. The plugin used to fill an empty
   * list in from the store — which made "unmute the last lane" impossible to
   * express, because an empty set and an unmentioned field looked identical.
   */
  mutedLanes?: string[];
  /** Lanes held across a reroll (TASK-044). */
  lockedLanes?: string[];
  /** Lanes soloed in the preview (TASK-043). Sent on every save, like the mutes. */
  soloedLanes?: string[];
  /**
   * The arrangement, when the producer has edited it (TASK-067).
   *
   * Absent for every project that never opened Song Mode, and for every song
   * still describable by its seed — see `songEdited`, and
   * `PluginSession::song` on the other side of the bridge.
   */
  song?: Song | null;
  songEdited?: boolean;
};

/**
 * The song document, as this module sees it.
 *
 * ⛔ **Reached through a registration seam rather than an import, and the
 * direction is the reason.** `song.ts` imports `useSession` — it has to, because
 * selecting a different artist must drop the arrangement, and that subscription
 * lives there. Importing `useSong` back here would close the loop, and a cycle
 * between two Zustand stores initialises in whichever order the bundler happens
 * to choose. `song.ts` registers itself at module load instead, so the
 * dependency still runs exactly one way.
 *
 * The defaults below are what a browser build sees: `song.ts` is imported by the
 * page, so in the app they are always replaced.
 */
type SongDocument = { song: Song | null; edited: boolean };

let readSongDocument: () => SongDocument = () => ({ song: null, edited: false });
let applySongDocument: (document: SongDocument) => void = () => {};

export function registerSongDocument(
  read: () => SongDocument,
  apply: (document: SongDocument) => void,
): void {
  readSongDocument = read;
  applySongDocument = apply;
}

/**
 * Record and save an edit made in a document that lives outside this store.
 *
 * ⛔ **Both halves, in one call, because the two must not be reachable
 * separately.** The session's own edits get them from two subscribers that
 * cannot be forgotten; the arrangement has no such subscriber here, so this is
 * the one door — and a caller that recorded without saving would build an undo
 * stack over a project file that never changed, while one that saved without
 * recording would put a state on disk that Ctrl+Z cannot reach.
 *
 * ⚠ **The `applying` check is a backstop, not a live guard.** Today a restore
 * reaches the arrangement through `applySongDocument`, which writes `useSong`
 * directly and never comes back through here — so nothing currently exercises
 * it. It stays because the failure it prevents is silent and total: recording a
 * restore would push an entry on every Ctrl+Z as it popped one, and the stack
 * could never be walked out of. The same reasoning as `Shared::set_running`'s
 * self-gate on the Rust side, which is also a backstop its callers already
 * honour.
 */
export function noteDocumentChange(): void {
  if (applying) return;
  useHistory.getState().record(snapshotOf(useSession.getState()));
  persist();
}

/**
 * Coalesces writes, because the seed box saves on every keystroke.
 *
 * The bridge is an HTTP round trip per call, and typing a six-digit seed would
 * otherwise be six of them. The host decides when to actually write the project
 * out, so there is nothing to be gained by being prompt here — only work to be
 * saved by not being.
 */
let saveTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Write now, cancelling any pending debounce.
 *
 * ⛔ The debounce is trailing-only, and two things do not wait for it: the host
 * serializes `#[persist]` state whenever *it* likes — project save, preset
 * save, freeze — and closing the editor destroys the page with the timer still
 * on it. Either inside the window loses the change silently, and the project
 * reopens on the previous value with nothing to explain it.
 */
function flush(): void {
  if (saveTimer === null) return;
  clearTimeout(saveTimer);
  saveTimer = null;
  send();
}

/**
 * Write now rather than in 300 ms, for a change the audio thread must hear.
 *
 * ⛔ **Not [`flush`]:** that only drains a debounce that is already pending, so
 * on the first change it does nothing at all. Most session writes can wait — the
 * host decides when to serialize anyway — but the lane mutes reach the audio
 * thread *through* this save, and half a beat of a lane still sounding after
 * the row has visibly dimmed reads as a broken control.
 */
function persistNow(): void {
  if (!isPlugin()) return;
  if (saveTimer !== null) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  send();
}

function send(): void {
  const state = useSession.getState();
  const session: Record<string, unknown> = Object.fromEntries(
    SAVED_FIELDS.map((key) => [key, state[key]]),
  );
  // ⛔ **Only an edited clip is sent, and it is not in `SAVED_FIELDS` for that
  // reason.** Every other field is small and unconditional; this one is the
  // whole pattern, and sending it for the unedited sessions that are most of
  // them would put a few hundred kilobytes of notes into every project file to
  // restore something the seed already describes exactly.
  // ⚠ **The whole map goes, not only the part that was drawn in.** `edited` is
  // one flag for the session, so it cannot say *which* of the five was edited —
  // and saving only the edited one would reopen the project with the other four
  // missing, which is the defect TASK-119 just closed arriving by a different
  // road. Five clips is still kilobytes against a project, and still nothing at
  // all for the sessions nobody has drawn in, which are most of them.
  if (state.edited && Object.keys(state.patterns).length > 0) session.patterns = state.patterns;
  // ⛔ **The same rule one document up (TASK-067).** An unedited arrangement is
  // reproducible by pressing Generate on the artist and seed already in this
  // payload, so storing it would be kilobytes of notes to restore something the
  // seed describes exactly. An *edited* one is not reproducible by anything, and
  // three handoffs running have recorded the symptom of leaving it out:
  // arranging a whole song and reopening the project lost all of it.
  const arrangement = readSongDocument();
  if (arrangement.edited && arrangement.song !== null) {
    session.song = arrangement.song;
    session.songEdited = true;
  }
  void invoke('save_session_state', { session }).catch(() => {
    // Losing a session write is not worth interrupting someone mid-beat. The
    // next change writes the whole session again anyway.
  });
}

function persist(): void {
  // Plugin only. A browser has nowhere to put this and no such command —
  // calling it would be a rejected promise per keystroke.
  if (!isPlugin()) return;

  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    send();
  }, 300);
}

/**
 * Put back what the host handed us when the project opened.
 *
 * **The pattern is not restored, because it is not saved** — the artist, seed
 * and pins are, and the engine is deterministic, so pressing Generate produces
 * the identical beat. `plugin/src/state.rs` explains why storing the inputs
 * beats storing a few hundred kilobytes of notes in someone's project file.
 */
function beginRestore(): Promise<SavedSession | null> {
  if (!isPlugin()) return Promise.resolve(null);
  return invoke<SavedSession>('session_state').catch(() => null);
}

async function apply(
  pending: Promise<SavedSession | null>,
  set: (partial: Partial<SessionState>) => void,
  get: () => SessionState,
): Promise<void> {
  const saved = await pending;
  if (!saved) return;

  // ⛔ The roster is clickable before this resolves — `init` awaits the roster
  // and the playback status first, and the user can pick an artist in between.
  // Writing anyway would replace the seed and pins under a selection they just
  // made, and leave `pendingArtist` naming an artist that is no longer chosen.
  // `loadDefaults` guards for exactly this reason; so does this.
  if (get().selectedId !== null) return;

  put(saved, set, get);
}

/**
 * Put a stored session's fields into the store.
 *
 * Shared by the project restore above and by loading a preset, which are the
 * same operation and differ only in whether they may overwrite a selection the
 * user has already made. Two copies of this would be two answers to "what does
 * a stored session set", and the pins are exactly where that drifts.
 *
 * Field by field rather than a spread: the plugin's pins are the engine's
 * six-field `SessionOverrides` and this store's are four, so a spread would put
 * `bars` and `halfTime` into a shape that has no room for them.
 */
function put(
  saved: SavedSession,
  set: (partial: Partial<SessionState>) => void,
  get: () => SessionState,
): void {
  // ⛔ **Only trusted when the session says it was edited.** A stored clip and
  // an unedited session together would mean regenerating anyway, and replaying
  // a pattern the seed can rebuild is how a project stops picking up engine
  // fixes for no benefit. `edited` is the flag, not "a pattern is present".
  // ⚠ **The legacy single clip is read into its own slot.** A project written
  // before TASK-119 carries `pattern` and no map; dropping it would lose the one
  // thing in a project file that cannot be regenerated. `pattern.part` says
  // where it belongs, so the migration is exact rather than a guess.
  const restored: PatternsByPart = !saved.edited
    ? {}
    : (saved.patterns ?? (saved.pattern ? { [saved.pattern.part]: saved.pattern } : {}));

  // ⛔ **One `set`, not two.** Every write here is recorded by the history
  // subscriber, so splitting the selection out of the rest made a single preset
  // load land as *two* undo entries — the first `Ctrl`+`Z` then stepped back to
  // a half-applied preset that was never on screen.
  const seed = saved.seed ?? '';

  set({
    seed,
    // ⛔ **The record comes back with the clips it belongs to.** Without this a
    // reopened project restored five parts and lost the harmonic plan they were
    // written against, so the first Generate afterwards started a *new* record
    // and that part no longer belonged with the four on screen — TASK-141
    // surviving exactly until the first reload.
    //
    // ⚠ Absent is `''`, which means "the next Generate starts a record". That
    // is right for a project written before this existed: those five clips were
    // generated from one seed anyway, so there is no record to recover.
    songSeed: saved.songSeed ?? '',
    // ⛔ **Absent means pinned, and the fallback is the whole compatibility
    // story.** Before the pin existed a stored seed *was* re-sent on every
    // Generate, so a project written then reopens reproducing its own beat —
    // which is US-004 at its strongest, since a project file is the most
    // deliberate way anyone ever keeps a seed. A stored `false` is honoured as
    // itself: that session never chose the seed it happens to be showing, so
    // acting on it would reintroduce exactly the defect this closes.
    // ⚠ An empty seed can never be pinned — there is nothing to reuse, and
    // `generate` would send `null` regardless. Deriving it here keeps the two
    // from disagreeing.
    seedPinned: seed !== '' && (saved.seedPinned ?? true),
    bars: saved.bars ?? get().bars,
    // Absent means on, matching the plugin's `auto_sync_default`: a project
    // written before the toggle existed must keep following its DAW.
    autoSync: saved.autoSync ?? true,
    mood: saved.mood ?? null,
    audioEnabled: saved.audioEnabled ?? true,
    mutedLanes: saved.mutedLanes ?? [],
    soloedLanes: saved.soloedLanes ?? [],
    lockedLanes: saved.lockedLanes ?? [],
    pins: {
      bpm: saved.pins?.bpm ?? null,
      keyRoot: saved.pins?.keyRoot ?? null,
      scale: saved.pins?.scale ?? null,
      swing: saved.pins?.swing ?? null,
      timeSigNum: saved.pins?.timeSigNum ?? null,
      timeSigDen: saved.pins?.timeSigDen ?? null,
    },
    // ⛔ The prompt asks about an artist switch, and a preset is not that
    // switch — leaving it up means answering it with "use theirs" wipes the
    // pins the preset just set. `applySnapshot` clears it for the same reason.
    pendingArtist: null,
    // ⛔ The incoming session's pattern has not been generated yet — it is
    // derived from the seed, on request. Leaving the old one up showed the
    // *previous* artist's beat under the new artist's name, which is the
    // readout-that-lies failure `loadDefaults` already guards against. Null on
    // a project restore too — unless the project carried an *edited* clip,
    // which the seed cannot reproduce and which is therefore the one thing
    // here that has to come back whole rather than be regenerated.
    patterns: restored,
    // ⚠ The wire carries one `edited` flag, so a restored project cannot say
    // *which* parts were edited. Every clip it saved is therefore treated as an
    // edit — which is right: they were only written because at least one was,
    // and marking them all preserves every one of them.
    editedParts: (Object.keys(restored) as Part[]).sort(),
    edited: Object.keys(restored).length > 0,
    // Set directly rather than through `select`, which would clear the pins as
    // a different artist's and raise the keep-or-adopt prompt. This is a
    // session arriving whole, not a switch.
    ...(saved.selectedId ? { selectedId: saved.selectedId } : {}),
  });

  // ⛔ **After the `set` above, not inside it.** Changing `selectedId` fires the
  // subscription in `song.ts` that clears the arrangement whenever the artist
  // moves — so a song restored first would be wiped by the very write that
  // restored the session it belongs to. Applied here it lands on the far side of
  // that, which is also the correct order for a preset load: the preset carries
  // no arrangement (see `presets::save_in`), so this clears whatever was open.
  //
  // Trusted only when the session says it was edited, for the same reason the
  // clip above is: an unedited song is regenerated from the seed rather than
  // replayed, so a project keeps picking up engine improvements.
  applySongDocument(
    saved.songEdited && saved.song
      ? { song: saved.song, edited: true }
      : { song: null, edited: false },
  );

  // ⛔ **Re-armed, because the `set` above already recorded a snapshot and the
  // line before this one made it stale.** The history subscriber fires inside
  // that `set`, reads the arrangement still on screen, and files it — and then
  // `applySongDocument` throws that arrangement away. `useHistory.present` then
  // named a record nobody could see, so one Ctrl+Y (or any session edit and a
  // single Ctrl+Z, which pushes the stale entry into `past`) resurrected the
  // arrangement a preset load had just deleted, alongside the preset's pins — a
  // state the producer was never in.
  //
  // ⚠ **`amend`, not `arm` and not `record`.** `arm` resets `past`, which would
  // make a preset load impossible to undo at all; `record` would push a second
  // entry and make it two steps, which `lands as one undo step, not two`
  // already guards. Amending corrects the entry that is current and leaves the
  // stack alone.
  //
  // ⚠ `init`'s restore path was safe only by accident — it calls `arm` *after*
  // `put`. `applyPreset` had no such correction, and this covers both.
  useHistory.getState().amend(snapshotOf(get()));

  if (saved.selectedId) {
    void loadDefaults(saved.selectedId, set, get);
  }
}

export const useSession = create<SessionState>((set, get) => ({
  roster: [],
  problems: [],
  rosterLoaded: false,

  selectedId: null,
  patterns: {},
  editedParts: [],
  bars: 4,
  seed: '',
  songSeed: '',
  // Nothing has been chosen, so nothing is held: the first Generate rolls.
  seedPinned: false,

  generating: false,
  error: null,

  playing: false,
  playhead: 0,
  playbackFailure: null,
  standalone: false,

  pins: NO_PINS,
  hostTempo: null,
  autoSync: true,
  mood: null,
  audioEnabled: true,
  mutedLanes: [],
  soloedLanes: [],
  lockedLanes: [],
  edited: false,
  defaults: null,
  pendingArtist: null,

  async refreshRoster() {
    try {
      const summary = await loadRoster();
      set({ roster: summary.entries, problems: summary.problems });
    } catch (error) {
      set({ error: reason(error) });
    }
  },

  async init() {
    const saved = beginRestore();

    try {
      const summary = await loadRoster();
      set({
        roster: summary.entries,
        problems: summary.problems,
        rosterLoaded: true,
      });
    } catch (error) {
      set({ rosterLoaded: true, error: reason(error) });
    }

    // Who owns the transport, and why it cannot be driven from here if it
    // cannot. Asked once at startup — a plugin cannot become a standalone
    // while it is running — so the buttons are honestly disabled rather than
    // failing on click.
    //
    // ⛔ **One command carrying both, because they are one fact.** They were
    // briefly two commands answered from the same source, which cost a second
    // serial round trip before the restored session could appear and gave the
    // page two flags that could drift into an enabled Play button whose tooltip
    // told the user to press play in their DAW.
    try {
      const status = await invoke<{
        standalone: boolean;
        reason: string | null;
        looping?: boolean;
      }>('playback_status');
      set({ standalone: status.standalone === true, playbackFailure: status.reason ?? null });
      // The plugin outlives the webview, so Loop is ITS state, not the page's.
      // The button defaulted to on and was never told otherwise: turn Loop off,
      // close the plugin window in a host, reopen it, and the control read lit
      // while the schedule was not looping.
      //
      // Optional because a browser mock and an older plugin both answer without
      // it; absent means keep the default rather than force one.
      if (typeof status.looping === 'boolean') useUi.getState().setLooping(status.looping);
    } catch (error) {
      // A shell with no transport commands at all is a dev-mode browser
      // session. Not a standalone either — there is no audio thread behind the
      // mock, so claiming the transport would be claiming it works.
      //
      // ⛔ **The reason is recorded, not swallowed.** This is asked exactly once
      // and never retried, so a dropped reply disables Play for the rest of the
      // session; leaving `playbackFailure` null meant the button was dead with
      // an empty tooltip and nothing anywhere saying why.
      set({ standalone: false, playbackFailure: reason(error) });
    }

    // Applied after the roster, because restoring a selection wants the entry
    // to exist for the rail to highlight and `loadDefaults` reads the dataset —
    // but *started* before it, since the read depends on nothing above and
    // `roster_summary` is the call that triggers the one-time dataset parse.
    // Waiting for it in series would queue a small lock read behind that.
    await apply(saved, set, get);

    // ⛔ Armed here and not at construction. The restore above writes the
    // session the host handed back, and a history that had been recording
    // would let Ctrl+Z step behind it onto an empty plugin — which reads as
    // the project having failed to load, not as an undo.
    useHistory.getState().arm(snapshotOf(get()));
  },

  select(id) {
    const { selectedId, pins, roster } = get();
    if (selectedId === id) return;

    // The old pattern belongs to the old artist. Keeping it on screen under a
    // new name would be the most convincing wrong thing the app could show.
    //
    // The pins are the deliberate exception: they are the user's, not the
    // artist's, so they survive the switch and the prompt asks about them.
    // There is nothing to ask on the first selection — the pins cannot be from
    // an artist when there was no artist.
    set({
      selectedId: id,
      // ⛔ A mood belongs to the artist it was picked for. Carrying it across
      // means the next Generate is refused by the engine — and on a style that
      // authors no modes the chip is not even rendered, so there is no control
      // on screen to clear it.
      mood: null,
      // ⛔ **All five, not just the one showing.** The old pattern belonged to
      // the old artist; so did the other four, and leaving any of them up would
      // show one artist's clips under another artist's name.
      patterns: {},
      editedParts: [],
      // ⛔ **The record belongs to the artist it was written for** (TASK-141).
      // The five clips above are being cleared precisely because they are the
      // old artist's; the harmonic plan they were written against is the same
      // fact one level down. Carrying it would make the next Generate join a
      // record whose other parts no longer exist — coherence with nothing.
      songSeed: '',
      error: null,
      defaults: null,
      pendingArtist:
        selectedId !== null && hasPins(pins)
          ? (roster.find((entry) => entry.id === id) ?? null)
          : null,
    });

    void loadDefaults(id, set, get);
  },

  setSeed(seed) {
    // ⛔ **Typing pins and clearing unpins**, so the common way in and the
    // common way out both work without anyone finding the lock. Pasting a seed
    // to get a beat back is US-004 and is meaningless unpinned; emptying the
    // box is the plainest possible way to say "surprise me", and it is what
    // the `random` placeholder already promises.
    const typed = seed.trim();
    set({ seed: typed, seedPinned: typed !== '' });
  },

  setSeedPinned(pinned) {
    // ⚠ There is nothing to hold when the box is empty. Refusing here rather
    // than in the button keeps the flag from ever contradicting the seed.
    if (pinned && get().seed === '') return;
    set({ seedPinned: pinned });
  },

  setBars(bars) {
    set({ bars });
  },

  setPin(field, value) {
    set({ pins: { ...get().pins, [field]: value } });
  },

  setAudioEnabled(on) {
    // Saved at once, like auto-sync: it is part of how a song was made, and a
    // producer who silenced the plugin expects it silent when they come back.
    set({ audioEnabled: on });
    persist();
  },

  setMood(mood) {
    // Saved like auto-sync and for the same reason: it is part of how a song
    // was made, not a transient view setting.
    set({ mood });
    persist();
  },

  setAutoSync(on) {
    // Saved immediately rather than on the next generation: it is part of how a
    // song was made, and a producer who turns it off and closes the project
    // expects it off when they come back.
    set({ autoSync: on });
    persist();
  },

  setLaneLocked(lane, locked) {
    const next = toggledLanes(get().lockedLanes, lane, locked);
    if (next === null) return;
    set({ lockedLanes: next });
    // ⚠ **The debounce is fine here, unlike the mute and the solo.** A lock
    // changes nothing the audio thread reads — it is consulted by `generate`,
    // on the page, at the moment the producer presses the button. There is no
    // window in which the plugin and the screen can disagree about it.
    persist();
  },

  setLaneSolo(lane, solo) {
    const next = toggledLanes(get().soloedLanes, lane, solo);
    if (next === null) return;
    set({ soloedLanes: next });
    // ⛔ **Now, not on the 300 ms debounce** — the same argument
    // `setLaneMuted` makes, and it bites harder here: a solo that arrives half a beat late
    // leaves every *other* lane audible after the row has visibly dimmed, so
    // the control looks like it did nothing at all.
    persistNow();
  },

  setLaneMuted(lane, muted) {
    const next = toggledLanes(get().mutedLanes, lane, muted);
    if (next === null) return;
    set({ mutedLanes: next });
    // ⛔ **Sent now, not on the 300 ms debounce.** The mask only reaches the
    // audio thread when the plugin adopts a saved session, so a debounced write
    // left the lane audibly playing for about half a beat at 120 BPM after the
    // row had already dimmed — and if that write failed the lane stayed audible
    // for good while the UI insisted it was muted. `flush()` also cancels the
    // pending timer, so this replaces the debounced write rather than racing it.
    persistNow();
  },

  async refreshHost() {
    try {
      const host = await invoke<HostSessionInfo>('host_session');
      // A tempo the host has not reported yet arrives as null, and that is a
      // different thing from 0 — the chip must fall back to the artist's value
      // rather than showing a tempo nothing is running at.
      const tempo = typeof host?.tempo === 'number' && host.tempo > 0 ? host.tempo : null;
      if (get().hostTempo !== tempo) set({ hostTempo: tempo });

      // ⛔ **The DAW owns whether time is running, and this poll is the only
      // thing that tells the page.** `playing` gates the playhead poll and
      // enables Stop; nothing in a *host* can start playback from this UI, so
      // without this the flag was permanently false — the marker never moved and
      // Stop was never clickable, with the whole transport silently inert.
      //
      // ⛔ **Except in the standalone, where the page is the one that decides.**
      // There `play()`/`pause()` write the flag optimistically and only then do
      // the round trip, because a button that waits half a second to look
      // pressed reads as a click that missed. This poll runs every 500 ms
      // against an atomic the audio thread republishes on its *next* block, so
      // a reply already in flight when the user clicks carries the pre-click
      // value — and writing it back un-pressed the button, froze the marker and
      // stayed wrong until the next poll, in both directions.
      if (!get().standalone) {
        const playing = host?.playing === true;
        if (get().playing !== playing) set({ playing });
      }
    } catch {
      // No host behind this UI — a browser, or a bridge that has no such
      // command. Not an error: there is simply no project tempo to follow, and
      // the artist's value stands.
      if (get().hostTempo !== null) set({ hostTempo: null });
    }
  },

  applyPreset(saved) {
    put(saved, set, get);
    // A preset that was not saved back would be forgotten the moment the host
    // wrote the project out — which is the next thing that happens after
    // someone loads one and presses Generate.
    persist();
  },

  keepPins() {
    set({ pendingArtist: null });
  },

  adoptDefaults() {
    set({ pins: NO_PINS, pendingArtist: null });
  },

  // ⛔⛔ **`standalone` dropped from this, deliberately (TASK-138).** It read
  // `standalone && playbackFailure === null`, which is what disabled Play inside
  // a DAW. The plugin drives its own *preview* transport now — `lib.rs` gates on
  // `host_playing || preview` and the host wins the moment its transport starts
  // — so a plugin window can offer Play like any other. Mike, 2026-08-04: *"i do
  // not want to just use Ableton's transpose play button."*
  //
  // ⚠ **`playbackFailure` still decides**, and it is now the only thing that
  // does: a missing output device or a kit that failed to decode is a real
  // refusal and the button must stay dark with the reason in its tooltip.
  canDriveTransport() {
    return get().playbackFailure === null;
  },

  async generate(part = 'drums') {
    const { selectedId, seed, seedPinned, songSeed, bars, generating, pins, mood, patterns } =
      get();
    if (!selectedId || generating) return;

    set({ generating: true, error: null });
    try {
      const pattern = await invoke<Pattern>('generate_pattern', {
        request: {
          styleId: selectedId,
          // ⛔ The bridge has taken a `part` since the five generators were
          // wired up, and the UI never sent one — so every tab that could
          // generate got drums. It is sent explicitly rather than left to the
          // bridge's default because "which part am I looking at" is the page's
          // question, not the engine's.
          part,
          bars,
          // ⛔⛔ **The pin decides, never the box's contents.** An unpinned box
          // is showing the seed the *engine* last picked, and re-sending that
          // is precisely the defect: press Generate twice, get the same beat
          // twice, forever. `null` means "pick one for me", which is what an
          // unpinned session is asking for every single time.
          // ⚠ Sending `""` would be a seed that fails to parse rather than an
          // absent one, which is why this is null and not the empty string.
          seed: seedPinned && seed !== '' ? seed : null,
          // ⛔ **The record, carried (TASK-141).** Unlike the seed above this is
          // sent whether or not anything is pinned, because it is not the
          // producer's choice — it is what makes the part being generated now
          // belong with the four generated before it. Null on the first
          // Generate of a session means "this one starts the record", and the
          // engine answers with the seed it chose.
          songSeed: songSeed !== '' ? songSeed : null,
          // ⛔ **Which drums are on screen, so a mirrored bass mirrors those.**
          // `bassline.rhythm = "mirror_kick"` — the roster default — copies the
          // kick's ticks one for one, and the engine was rebuilding a reference
          // kit at the *record's* seed while the drums the producer can see came
          // from their own take. Boom-bap went from 13 of 13 bass notes on a
          // real kick to 9 of 13; uk-drill to 1 of 14.
          // ⚠ Sent on every part, not only bass: only `Part::Bass` reads it, and
          // a conditional here would be one more thing to keep in agreement with
          // the engine. Null before the drums have been generated, which is the
          // honest answer — there is no take to mirror yet.
          //
          // ⛔⛔ **...and null again once the session has moved under them.** The
          // engine rebuilds the kick from `(model, ctx, seed)`, so the seed only
          // reproduces the drums on screen while the *context* still matches.
          // Generate drums at 4 bars, drag the bars chip to 8, then generate the
          // bass: the kick is rebuilt over 8 bars, its ticks differ from the
          // clip the producer is looking at, and the mirrored bass lands on
          // kicks nobody is playing — the narrower form of the very defect this
          // field was added to close. `mirrorable` compares what the drums were
          // built with against what this generation will use, and sending null
          // falls back to the record's canonical kit, which is at least a
          // coherent answer rather than a confidently wrong one.
          drumsSeed: mirrorableDrumsSeed(patterns.drums, { bars, songSeed, pins, mood }),
          // Every unpinned field goes as null, which serde reads as absent —
          // the artist's own value then stands (FR-002).
          session: pins,
          // Null is "Any", which the engine answers by picking from the seed
          // rather than by generating without a mode (TASK-040V).
          mood,
        },
      });
      // Show the seed that was actually used, so the chip can be copied even
      // when the user never typed one (US-004).
      //
      // ⛔ **The echo is a readout, and `seedPinned` is deliberately not set.**
      // It was the absence of that distinction that made Generate return one
      // beat per artist forever — see `seedPinned` on the state type. A
      // producer who wants to keep what just came back presses the lock, or
      // copies the number; what they must never get is the app quietly
      // deciding for them that they meant to.
      //
      // ⚠ **Coherence across parts is now the pin's job, and that is a real
      // change in behaviour.** `parts.rs` guarantees the five agree only on a
      // shared seed, and generating drums and then a melody separately used to
      // share one *because* of this echo. Unpinned, each press now rolls, so
      // the two are written against different records. The two ways to get a
      // matching set are `generateAll` — which shares one fresh seed across all
      // five on purpose — and pinning the seed after the part you want to build
      // around. Anything cleverer (reuse across *different* parts, roll on the
      // *same* one) makes the rule depend on what you pressed last, which is
      // not a rule anybody can hold in their head while making a beat.
      //
      // ⚠ **Only this part's slot is replaced.** The other four keep their
      // existing object references, so their editors do not re-render and the
      // undo stack's shared-reference model still holds.
      //
      // `edited: false` — a fresh generation *is* the seed's own output again,
      // so the project goes back to storing the request rather than the clips.
      set((state) => {
        // ⛔ **Only *this* part stops being an edit.** `edited` used to be
        // cleared outright here, and because `send()` uses it to decide whether
        // *any* clip is saved, regenerating one part wrote the project with no
        // clips at all — silently deleting every other part's hand edits.
        // ⛔ **The locks are applied here, against the slot this generation is
        // replacing.** Not against `get()` outside the updater — zustand hands
        // the current state in, and reading it separately would race a second
        // Generate that landed between the request and its answer.
        //
        // ⚠ **Not during a recall.** `recallVariation` regenerates in order to
        // *reproduce* a take, and splicing whatever is locked right now into it
        // returns a hybrid of that take and this one — while the nav goes on
        // reporting the take the producer asked for. A clip that never existed,
        // labelled as one that did.
        const held = recalling
          ? pattern
          : withLocks(pattern, state.patterns[part], state.lockedLanes);
        // ⛔⛔ **A held lane is an edit, and clearing the flag lost it.** This
        // said `withoutEdit` unconditionally — a fresh generation *is* the
        // seed's own output again — which stopped being true the moment
        // `withLocks` began splicing a previous take's lane in. `send()` reads
        // `edited` to decide whether any clip is written at all, so locking the
        // hats you drew and pressing Generate saved a project with no clips in
        // it: reopening regenerated from the seed and the locked lane was gone
        // with no message. Identity is the test — `withLocks` returns `next`
        // unchanged when no lock landed — so the flag follows what actually
        // happened rather than what was asked for.
        const editedParts =
          held === pattern
            ? withoutEdit(state.editedParts, part)
            : withEdit(state.editedParts, part);
        // ⛔ **The variation history is appended here, inside the updater, for
        // the reason the locks are: this is where the generation *lands*.**
        // Recording it outside would let a second Generate that resolved first
        // write its entry second, so the log would disagree with the order the
        // producer pressed things in — which is the one thing a history is for.
        if (!recalling) {
          useVariations
            .getState()
            .record(entryFor(pattern, { mood: state.mood, pins: state.pins }, Date.now()));
        }
        return {
          patterns: { ...state.patterns, [part]: held },
          seed: pattern.seed,
          // ⚠ Held so the *next* part joins this record rather than starting
          // its own. Nothing shows it and nothing asks the producer about it —
          // coherent parts should not need a control to switch on.
          songSeed: pattern.songSeed,
          generating: false,
          editedParts,
          edited: editedParts.length > 0,
        };
      });
    } catch (error) {
      set({ generating: false, error: reason(error) });
    }
  },

  async generateAll() {
    const { selectedId, bars, generating, pins, mood, seedPinned } = get();
    if (!selectedId || generating) return;

    set({ generating: true, error: null });

    // ⛔ **Accumulated locally and written once.** Five `set` calls would be
    // five history entries for one deliberate act, five arms of the audio
    // thread, and four renders of a half-filled session.
    const filled: PatternsByPart = { ...get().patterns };
    // Starts from what is already edited rather than from empty, so a part this
    // run never touches — one the style refuses — keeps the hand edits it has.
    let editedAfter: Part[] = get().editedParts;
    // ⛔ **One fresh seed for the set, unless the producer pinned one.** Empty
    // means "pick one for me", and once the engine has, every remaining part
    // must be given the same one — see `generateAll` on the type above for why
    // that is correctness rather than tidiness. Starting from the *unpinned*
    // box instead would hand every press the seed the last run echoed back,
    // so "Generate all" would rebuild the identical record every time.
    //
    // ⚠ Local, and it does not touch `seedPinned`. The shared seed is how these
    // five agree with each other; it is not a choice the producer made.
    let seed = seedPinned ? get().seed : '';
    // ⛔ **The record is carried through the loop too, and it was not.**
    // `generateAll` was written before TASK-141 and never learned about it: it
    // neither sent `songSeed` nor wrote it back. Two failures fell out of that,
    // and "Generate all, then reroll one part" is the commonest workflow there
    // is. (a) After Generate All the store's record was still `''`, so the next
    // single Generate started a *new* record and that one part no longer
    // belonged with the other four. (b) A record left over from an earlier
    // single Generate survived the run — the engine rolled a fresh one for the
    // five, the store kept the stale one, and every press after that joined a
    // record nothing on screen belonged to.
    let record = get().songSeed;
    const refused: string[] = [];
    // ⚠ Tracked by identity rather than by counting keys — see the write below.
    let landed = false;

    for (const part of GENERATED_PARTS) {
      try {
        const pattern = await invoke<Pattern>('generate_pattern', {
          request: {
            styleId: selectedId,
            part,
            bars,
            seed: seed === '' ? null : seed,
            songSeed: record === '' ? null : record,
            // `GENERATED_PARTS` puts drums first, so by the time this loop
            // reaches the bass the take it must mirror is already in `filled`.
            // ⚠ Today every part of one "Generate all" shares a seed, so this is
            // the same number the bass is being generated at — it is sent
            // anyway, because the loop's order is what makes that true and a
            // silent dependency on it is how the single-Generate path came to
            // disagree with this one in the first place.
            // ⚠ Through the same guard the single path uses: the drums in
            // `filled` may be left over from a previous run under a different
            // session, in which case their seed no longer names them.
            drumsSeed: mirrorableDrumsSeed(filled.drums, {
              bars,
              songSeed: record,
              pins,
              mood,
            }),
            session: pins,
            mood,
          },
        });

        // ⛔ **The artist may have changed while this was in flight.** `select`
        // clears the slots and swaps `selectedId`; without this check the loop
        // would go on to write the *previous* artist's five clips under the new
        // one's name and arm one of them — which `select`'s own comment calls
        // the most convincing wrong thing the app could show.
        if (get().selectedId !== selectedId) {
          set({ generating: false });
          return;
        }

        // ⛔⛔ **The locks, here too.** They were applied in `generate` only,
        // so locking the kick and pressing Shift+G lost the lock without a
        // word — a rule installed at one door rather than at the seam. Both
        // doors now go through `withLocks`, and the e2e asserts the Shift+G
        // path specifically because that is the one that was wrong.
        const held = withLocks(pattern, get().patterns[part], get().lockedLanes);
        filled[part] = held;
        // ⛔ **And the same identity test `generate` uses.** A part whose lock
        // landed is no longer reproducible from the seed, so it has to keep
        // being written into the project; one that generated cleanly stops
        // being an edit. Tracked here and applied once at the end, because a
        // `set` per part is five history entries for one deliberate act.
        editedAfter =
          held === pattern ? withoutEdit(editedAfter, part) : withEdit(editedAfter, part);
        seed = pattern.seed;
        record = pattern.songSeed;
        landed = true;
        // ⛔ **A take is a take however it was made.** `generateAll` does not
        // go through `generate`, so until TASK-046's keyboard test noticed, one
        // press of Generate All produced five takes the variation history had
        // never heard of — and stepping back afterwards skipped straight past
        // them to whatever came before. Recorded per part, because the counter
        // is per part.
        useVariations
          .getState()
          .record(entryFor(pattern, { mood: get().mood, pins: get().pins }, Date.now()));
      } catch (error) {
        // ⛔ **A refused part is not a failed run, and treating it as one was
        // the defect.** A style whose 808 *is* the bassline authors no separate
        // bass part on purpose (FR-007) and the engine says so by refusing —
        // so on Drake, and on most of the trap roster, the fifth request always
        // fails. Aborting there threw away the four parts that had already come
        // back and the seed that made them agree, which is how "Generate all"
        // came to do nothing at all on a flagship artist.
        refused.push(reason(error));
      }
    }

    if (get().selectedId !== selectedId) {
      set({ generating: false });
      return;
    }

    // Nothing came back at all — that is a real failure and keeps the error it
    // reported. Anything else is a partial success, and the parts that
    // generated are worth more than the tidiness of refusing all five.
    //
    // ⛔ **Identity, not a key count.** `filled` starts as a copy of the
    // existing slots, so once every generatable part has one the count can
    // never grow — and the *second* press of Generate All on a style that
    // refuses a part (Drake, and most of the trap roster, whose 808 is the
    // bassline) compared 4 against 4, decided nothing had landed, and threw
    // away four freshly generated clips with an error banner. Which is the
    // exact symptom the comment below says was fixed, returning on press two.
    if (!landed && refused.length > 0) {
      set({ generating: false, error: refused[0] });
      return;
    }

    set({
      patterns: filled,
      seed,
      songSeed: record,
      generating: false,
      // Every part that landed cleanly is the seed's own output again — but a
      // part whose lock landed is not, and a part that was refused keeps
      // whatever edit state it already had. `editedAfter` carries both.
      editedParts: editedAfter,
      edited: editedAfter.length > 0,
      // ⚠ The refusals are still worth saying — a producer who asked for five
      // parts and got four should be told which one the style does not have,
      // rather than left to notice the empty tab later.
      error: refused.length > 0 ? refused[0] : null,
    });
  },

  clearPart(part) {
    // ⚠ A no-op when the slot is already empty, so clearing twice does not push
    // a second undo entry for a button press that changed nothing.
    if (get().patterns[part] === undefined) return;
    set((state) => {
      const patterns = { ...state.patterns };
      delete patterns[part];
      // ⛔ Clearing the one clip that was edited must also clear the *claim*
      // that the session is edited, or `send()` goes on writing the remaining
      // seed-reproducible clips into the project — which then replays them
      // verbatim on every reopen instead of regenerating, and the session never
      // picks up a later engine fix.
      const editedParts = withoutEdit(state.editedParts, part);
      return { patterns, editedParts, edited: editedParts.length > 0 };
    });
  },

  clearAll() {
    if (Object.keys(get().patterns).length === 0) return;
    set({ patterns: {}, editedParts: [], edited: false });
  },

  openClip(pattern, part) {
    // ⛔ **`edited` is set, and it is not a guess.** A clip lifted out of an
    // arrangement is not what *this session's* seed produces — the session's
    // seed makes a four-bar loop for whichever part is showing, and this is one
    // section's clip out of a song. Left false, the next save would drop it and
    // the next Generate would silently replace it.
    // ⛔ **The tab moves first, and the order is the fix rather than a
    // preference.** The `patterns` subscriber runs synchronously inside `set`
    // and defers to `armCurrentPattern`, which reads the *active tab* — so with
    // the tab still on Song, `patternForTab` answered `null` (Song is not a
    // part) and fired a `disarm`. `setActiveTab` then unmounted `SongTimeline`,
    // whose cleanup armed the clip: two fire-and-forget round trips in flight,
    // and if the `disarm` resolved second the drilled-in clip was silent. Moving
    // the tab first means the subscriber already sees the part it is arming.
    useUi.getState().setActiveTab(part);
    set((state) => ({
      patterns: { ...state.patterns, [part]: pattern },
      editedParts: withEdit(state.editedParts, part),
      edited: true,
    }));
  },

  editPattern(next) {
    // Reference-compared rather than deep-compared: every edit in
    // `PianoRoll/notes.ts` returns the *same* object when it changes nothing
    // (a move clamped to zero, a delete with an empty selection), so this
    // filters those out for free and keeps a no-op gesture off the undo stack.
    // ⛔ **The part comes off the pattern, not off the active tab.** They agree
    // today, and tying the write to the tab would make them disagree the first
    // time an edit is applied while the producer has clicked elsewhere — which
    // would drop a melody edit into the bass slot.
    if (get().patterns[next.part] === next) return;
    // ⛔ **`edited` latches here and nowhere else.** From this call on the seed
    // no longer describes what is on screen, so the project has to store the
    // clip rather than the request that made it — see `edited`'s own note.
    set((state) => ({
      patterns: { ...state.patterns, [next.part]: next },
      editedParts: withEdit(state.editedParts, next.part),
      edited: true,
    }));

    // ⛔ **The edit has to reach the audio thread, or the preview keeps playing
    // the notes that were there before it.** Nothing else does this: the
    // schedule is armed in `editor.rs` from any reply that is a `Pattern`, and
    // a purely local edit produces no reply at all — so a producer moved a
    // note, watched it move, pressed play and heard the old one.
    //
    // Fire-and-forget for the same reason the session save is: losing one is
    // not worth interrupting someone mid-beat, and the next edit sends the
    // whole pattern again. Outside the plugin there is no audio thread to tell.
    // Arming the audio thread and telling the project are the subscriber's job
    // — see `clipChanged` below. Doing them here would be three call sites for
    // one rule, and `applySnapshot` would still be the one that forgot.
  },

  async seek(progress) {
    const to = Math.min(1, Math.max(0, progress));
    // Moved locally first so the marker lands under the pointer on the same
    // frame as the click. The audio thread is a block behind at worst, and a
    // marker that waits for a round trip reads as a click that missed.
    set({ playhead: to });
    try {
      await invoke('seek', { progress: to });
    } catch {
      // No audio thread behind the mock; the local move still stands.
    }
  },

  async play() {
    // ⛔ `playing` is set optimistically rather than waited for. In the
    // standalone the flag the audio thread reads is the same one `refreshHost`
    // reports back, but that poll is up to 500 ms away — and a Play button
    // that stays un-pressed for half a second reads as a click that missed.
    // `error: null` on the way in rather than on success: the round trip may
    // fail and overwrite it, and a stale failure banner sitting over a running
    // transport is the contradiction this clears.
    set({ playing: true, error: null });
    try {
      await invoke('transport_play');
    } catch (error) {
      set({ playing: false, error: reason(error) });
    }
  },

  async pause() {
    // ⛔ The playhead is deliberately left where it is. That is the entire
    // difference between Pause and Stop, and it is the semantics the audio
    // thread already has: pausing stops advancing the schedule, stopping seeks
    // it back to zero.
    set({ playing: false, error: null });
    try {
      await invoke('transport_pause');
    } catch (error) {
      // ⛔ Rolled back, like `play()`. Without this the store said paused while
      // the audio thread kept advancing — a transport reported as held that is
      // audibly still running, which is worse than the button not responding.
      set({ playing: true, error: reason(error) });
    }
  },

  async stop() {
    try {
      await invoke('stop_playback');
    } catch {
      // A stop that fails still means the user wants it stopped; showing an
      // error for it would be noise.
    }
    set({ playing: false, playhead: 0 });
  },

  async recallVariation(entry) {
    // ⛔ **The seed is *pinned* on the way in.** `generate` sends `null`
    // unless the seed is pinned — that is the fix for "Generate returns the same
    // beat every press" — so recalling without pinning would draw a fresh seed
    // and land somewhere the producer has never been.
    //
    // ⛔⛔ **And so is everything else the take was written against.** This
    // restored the artist, seed, bars, mood and *pins* — but the pins are what
    // was asked for, and `bpm`/`keyRoot`/`scale`/`timeSig` on the entry are what
    // was actually **used**, which is not the same thing and is exactly why the
    // entry stores both. Unpinned, a take made at 140 came back at whatever the
    // session had drifted to while `VariationNav` went on displaying 140 off the
    // entry: the readout and the notes disagreeing about the take the producer
    // had just asked for. The resolved values are pinned here so the clip that
    // comes back is the clip the nav is describing — and being pins, the session
    // chips show every one of them, so the state a recall leaves behind is on
    // screen rather than hidden.
    const previous = get().selectedId;
    set({
      selectedId: entry.artistId,
      seed: entry.seed,
      songSeed: entry.songSeed,
      seedPinned: true,
      bars: entry.bars,
      pins: {
        ...entry.pins,
        bpm: entry.bpm,
        keyRoot: entry.keyRoot,
        scale: entry.scale,
        timeSigNum: entry.timeSigNum,
        timeSigDen: entry.timeSigDen,
      },
      mood: entry.mood,
    });

    // ⛔⛔ **A different artist needs everything a `select` does.** This wrote
    // `selectedId` with a bare `set`, so `defaults` still held the *previous*
    // artist's — and `defaults` is what the ARTIST pane's "tends to" line reads
    // and what every unpinned field falls back on. The roster highlighted one
    // artist while the panel beside it described another. The other four slots
    // were left up too, showing that artist's clips under this one's name.
    //
    // ⚠ **Not `select()` itself**, which raises the keep-or-adopt prompt when
    // pins are set — and they always are here, because the line above just set
    // them. Asking "which artist's session wins?" in the middle of stepping
    // backwards through your own history is a question with no meaning.
    if (previous !== entry.artistId) {
      set({ patterns: {}, editedParts: [], edited: false, defaults: null, error: null });
      void loadDefaults(entry.artistId, set, get);
    }
    // ⚠ **Regenerated rather than restored from stored notes**, which is what
    // makes an entry tens of bytes: the engine is deterministic, so the same
    // artist, seed, bars and pins rebuild the take exactly.
    recalling = true;
    try {
      await get().generate(entry.part);
    } finally {
      // ⛔ `finally`, so a generation that throws does not leave the flag set
      // and silently stop logging every take from then on.
      recalling = false;
    }
  },

  undo() {
    const snapshot = useHistory.getState().undo();
    if (snapshot !== null) applySnapshot(snapshot, set, get);
  },

  redo() {
    const snapshot = useHistory.getState().redo();
    if (snapshot !== null) applySnapshot(snapshot, set, get);
  },
}));

/**
 * Record every document change as an undo step (FMM-U01).
 *
 * A subscription for the same reason the save below is one: opting in per
 * action is a line to remember in every future action, and it was already wrong
 * in both directions once. This cannot be forgotten, and it sees the seed the
 * engine writes back after a generation for free.
 *
 * ⛔ Not gated on `isPlugin()`, unlike the save. Undo belongs to the app in
 * every shell it runs in — the standalone and the desktop build included.
 */
useSession.subscribe((state) => {
  if (applying) return;
  useHistory.getState().record(snapshotOf(state));
});

/**
 * Re-arm when the producer changes tabs.
 *
 * ⛔ **`armCurrentPattern` reads the active tab, and nothing was watching it.**
 * The `patterns` subscriber below only fires when a slot changes, and
 * `setActiveTab` writes to `useUi` — so generating a melody and then clicking
 * back to Drums left the *melody* on the audio thread while the drum grid was
 * on screen with its playhead crawling across it. That is the readout-that-lies
 * failure `armCurrentPattern`'s own doc says it exists to prevent, and under the
 * single-slot shape it could not happen because there was only one clip to arm.
 *
 * ⚠ Song is excluded: `SongTimeline`'s own mount effect arms the arrangement and
 * its cleanup hands the transport back, so arming here would race it.
 */
useUi.subscribe((state, prev) => {
  if (state.activeTab === prev.activeTab || state.activeTab === 'song') return;
  armCurrentPattern();
});

/**
 * Save the session whenever the user changes it.
 *
 * A subscription rather than a `persist()` call at the end of each mutating
 * action. Opt-in was one line per action to remember, and it was already wrong
 * in both directions: `keepPins` called it while changing nothing that is
 * saved, and `generate` needed its own call precisely *because* an opt-in
 * cannot notice that the engine wrote a fresh seed back into the store. A
 * subscriber sees that for free, and the next action to touch these fields
 * cannot forget.
 *
 * Reference equality is enough for `pins`: every writer replaces the object.
 */
if (isPlugin()) {
  useSession.subscribe((state, prev) => {
    // ⛔ Compared against the same list `send()` writes. Leaving a field out
    // meant an undone or redone change never reached the project — the session
    // reopened contradicting what the UI had just shown — and that happened
    // three times while these were two hand-maintained lists.
    if (SAVED_FIELDS.every((key) => state[key] === prev[key])) return;
    persist();
  });

  /**
   * The clip itself, which `SAVED_FIELDS` deliberately does not cover.
   *
   * ⛔ **A subscriber, not two lines at each door, and that is the whole point.**
   * A new pattern has to reach the audio thread — `editor.rs` arms the schedule
   * from any reply that *is* a `Pattern`, and a local edit produces no reply at
   * all, so a producer moved a note, watched it move, pressed play and heard the
   * old one. There are four writers of `pattern`: `generate`, `editPattern`, the
   * project restore, and `applySnapshot`. When this was a ritual copied into
   * each, `applySnapshot` was the one that had neither — so **undo showed the
   * old notes while the audio thread kept playing the new ones**, and an undo
   * between two edits never reached the project file either.
   *
   * ⛔ The save is conditional on `edited` and the arm is not: an unedited clip
   * is regenerated from its seed and is not worth a byte in the project, but
   * every clip that reaches the screen has to be the one that plays.
   */
  // The page is going away — `pagehide` is the last event a webview reliably
  // delivers, and `visibilitychange` covers a host that hides the editor
  // without destroying it. Both are cheap no-ops when nothing is pending.
  window.addEventListener('pagehide', flush);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flush();
  });
}

/**
 * Show the Stems panel once this session has generated something.
 *
 * ⛔⛔ **Mike found this in Ableton, 2026-08-06:** *"the stems panel should be
 * visible if you have done a generation so that way you can ensure that you can
 * drag it in no matter what right away."* The panel remembers being collapsed
 * across reloads, and it holds the only way to get a pattern out of the plugin —
 * so a producer who collapsed it once had no route to the drag rows and nothing
 * on screen suggesting they existed.
 *
 * ⛔ **Not gated on `isPlugin()`, unlike the save below.** This is a UI rule, and
 * gating it would make it untestable in exactly the place it is tested. The
 * browser build has no drag source, but it has the Export buttons in the same
 * panel and the same reason to show them.
 *
 * ⚠ Fires on any write that leaves a pattern in the store rather than on
 * `generate` alone — restoring a project and drilling into a song clip both
 * arrive here without going through it. `revealStems` is idempotent, so the
 * repetition costs nothing and a deliberate collapse afterwards survives.
 */
useSession.subscribe((state) => {
  if (Object.keys(state.patterns).length === 0) return;
  useUi.getState().revealStems();
});

useSession.subscribe((state, prev) => {
  if (state.patterns === prev.patterns || !isPlugin()) return;
  // ⛔ **Arm the tab that is showing, not "the pattern that changed".** There
  // are five slots and one schedule, so the transport can only hold one of
  // them, and the one it must hold is the one the producer is looking at —
  // otherwise generating a bassline on the Bass tab while the Drums tab is open
  // would silently swap what Play does. `armCurrentPattern` owns that choice;
  // this defers to it rather than keeping a second copy of the rule.
  //
  // ⚠ Hearing all five at once is TASK-120's, and it needs a merge the audio
  // thread does not have yet. Until then Play is honest about being one part.
  armCurrentPattern();
  if (state.edited) persist();
});

/**
 * Put the clip back on the audio thread, unchanged.
 *
 * ⛔ **The Song tab arms the whole arrangement, and there is only one
 * schedule** — so leaving Song Mode has to give the transport back what the
 * generator tabs are showing. Without this, switching to Song, then back to
 * Drums, and pressing Play would play the song: the marker would be over the
 * roll and the sound would be the record, which is the readout-that-lies
 * failure at its worst because both halves look right on their own.
 *
 * The subscriber above cannot serve this — it fires on the pattern *changing*,
 * and nothing about a tab switch changes it.
 */
export function armCurrentPattern(): void {
  if (!isPlugin()) return;
  // ⛔⛔ **Every generator that is switched on, not the visible tab's clip
  // (TASK-127).** Mike, 2026-08-06: *"i want to be able to play the generators
  // all at once or separately, they should be able to be toggled on and off for
  // each generator."* The bridge merges these into the one `Pattern` a schedule
  // can hold; one part on is the ordinary solo case.
  //
  // ⚠ **This replaces "arm whatever tab you are looking at", deliberately.**
  // That rule existed because one clip could be armed at a time, so the visible
  // one was the only defensible choice. With toggles the producer says which
  // parts sound, and switching tabs no longer changes what Play does — which is
  // the point: they can watch the melody while hearing it over the drums.
  //
  // ⚠ Song Mode is untouched and falls through to the disarm below:
  // `TAB_PART.song` is `null`, and `SongTimeline` arms the arrangement itself.
  if (TAB_PART[useUi.getState().activeTab] !== null) {
    const on = armedClips(
      useSession.getState().patterns,
      useUi.getState().partsOff,
      useUi.getState().activeTab,
    );
    if (on.length > 0) {
      void invoke('arm_pattern', { patterns: on }).catch(() => {});
      return;
    }
    // Everything generated is switched off, or nothing is generated. Fall
    // through: the transport must have nothing rather than the last thing.
  }
  // ⛔ **A null pattern must *disarm*, not return.** This is the common case,
  // not an edge one: a session that has only ever used Song Mode has no clip at
  // all — `generate_song` answers with a `Song`, which `editor.rs` does not
  // deserialize as a `Pattern`, so nothing ever armed one. Returning here left
  // the whole arrangement on the transport, so a producer could open Song Mode,
  // click the Drums tab, see the empty "ready to generate" state, press Play and
  // hear the entire record. That is the exact readout-that-lies failure this
  // function exists to prevent, in the one case it was silently skipping.
  void invoke('disarm').catch(() => {});
}

/**
 * Follow the playhead the audio thread publishes.
 *
 * Returns a no-op outside the plugin: a browser has no audio thread, so there
 * is nothing advancing to follow.
 */
export function subscribeToPlayhead(): () => void {
  // ⛔ **The plugin has no event system to push this, so it polls** (TASK-041T).
  // The bridge is an HTTP round trip over the custom protocol — wry's IPC is
  // one-way, and a window parented into Ableton never gets the frame tick a push
  // would need. That is the same constraint that made every other command a
  // request/response, and it is why this is a poll rather than a listener.
  //
  // At frame rate against an atomic the audio thread already writes every block,
  // so the marker moves with the tempo without the audio thread ever waiting for
  // the page. Stopped when the editor closes, like every other subscription here.
  if (!isPlugin()) return () => {};

  let live = true;
  const tick = async () => {
    if (!live) return;
    // ⛔ Only while something is playing. An idle editor is the normal state
    // and it must cost nothing — polling regardless was a round trip per
    // frame, forever, to read a number that cannot change.
    if (!useSession.getState().playing) {
      schedule();
      return;
    }
    try {
      const position = await invoke<number>('playhead');
      // ⛔ Only write when it moved. `set` on every frame would re-render the
      // grid sixty times a second whether or not anything changed, and the
      // playhead line is a CSS variable precisely so it does not have to.
      if (useSession.getState().playhead !== position) {
        useSession.setState({ playhead: position });
      }
    } catch {
      // A dropped poll is a dropped frame of the marker, and the next one
      // fixes it. Reporting it would put an error on screen for nothing.
    }
    schedule();
  };
  // 30 Hz, which is the rate `App.tsx` already documents. rAF would be 60 and
  // buys nothing: the marker is one CSS variable, and the pattern it walks is
  // seconds long.
  const schedule = () => {
    if (live) window.setTimeout(() => void tick(), 33);
  };
  void tick();
  return () => {
    live = false;
  };
}
