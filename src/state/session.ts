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
import { useHistory, type Snapshot } from './history';

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

/** Bar counts the UI offers. Four is the default a pattern is demonstrated at. */
export const BAR_CHOICES = [2, 4, 8] as const;

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
type DocumentFields = 'pattern' | 'song' | 'songEdited';

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
  'bars',
  'pins',
  'autoSync',
  'mood',
  'audioEnabled',
  'mutedLanes',
  'edited',
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

type SessionState = {
  roster: RosterEntry[];
  problems: DatasetProblem[];
  rosterLoaded: boolean;

  selectedId: string | null;
  pattern: Pattern | null;
  bars: number;
  /**
   * The seed to generate with, as typed. A string because a u64 does not
   * survive a JSON number, which is the same reason `Pattern.seed` is one.
   */
  seed: string;

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
  select: (id: string) => void;
  setSeed: (seed: string) => void;
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
  editPattern: (next: Pattern) => void;
  /** Run our own transport. Standalone only — in a host this is the DAW's. */
  play: () => Promise<void>;
  /** Hold it where it is. Standalone only, for the same reason. */
  pause: () => Promise<void>;
  stop: () => Promise<void>;

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
  const { selectedId, seed, bars, pins, autoSync, mood, audioEnabled, mutedLanes, edited } =
    state;
  // The arrangement lives in its own store and is read through the seam, for
  // the same reason `send()` reads it there — see `registerSongDocument`.
  const arrangement = readSongDocument();
  return {
    selectedId,
    seed,
    bars,
    pins,
    autoSync,
    mood,
    audioEnabled,
    mutedLanes,
    edited,
    pattern: state.pattern,
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
  bars: number | null;
  pins: Partial<SessionPins> | null;
  /**
   * The clip as edited, when the seed no longer describes it (TASK-041).
   *
   * Absent for every session nobody has drawn in — see `edited` on the store,
   * and `PluginSession::pattern` on the other side of the bridge.
   */
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
  if (state.edited && state.pattern !== null) session.pattern = state.pattern;
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
  const restored = saved.edited && saved.pattern ? saved.pattern : null;

  // ⛔ **One `set`, not two.** Every write here is recorded by the history
  // subscriber, so splitting the selection out of the rest made a single preset
  // load land as *two* undo entries — the first `Ctrl`+`Z` then stepped back to
  // a half-applied preset that was never on screen.
  set({
    seed: saved.seed ?? '',
    bars: saved.bars ?? get().bars,
    // Absent means on, matching the plugin's `auto_sync_default`: a project
    // written before the toggle existed must keep following its DAW.
    autoSync: saved.autoSync ?? true,
    mood: saved.mood ?? null,
    audioEnabled: saved.audioEnabled ?? true,
    mutedLanes: saved.mutedLanes ?? [],
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
    pattern: restored,
    edited: restored !== null,
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

  if (saved.selectedId) {
    void loadDefaults(saved.selectedId, set, get);
  }
}

export const useSession = create<SessionState>((set, get) => ({
  roster: [],
  problems: [],
  rosterLoaded: false,

  selectedId: null,
  pattern: null,
  bars: 4,
  seed: '',

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
  edited: false,
  defaults: null,
  pendingArtist: null,

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
      const status = await invoke<{ standalone: boolean; reason: string | null }>(
        'playback_status',
      );
      set({ standalone: status.standalone === true, playbackFailure: status.reason ?? null });
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
      pattern: null,
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
    set({ seed: seed.trim() });
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

  setLaneMuted(lane, muted) {
    const current = get().mutedLanes;
    if (current.includes(lane) === muted) return;
    // ⛔ Sorted, so the list is a set rather than a history of the order they
    // were clicked in. Two projects that mute the same two lanes must save the
    // same bytes, or an undo entry and a project diff both record a change
    // nobody made.
    const next = muted
      ? [...current, lane].sort()
      : current.filter((muted_lane) => muted_lane !== lane);
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

  canDriveTransport() {
    const { standalone, playbackFailure } = get();
    return standalone && playbackFailure === null;
  },

  async generate(part = 'drums') {
    const { selectedId, seed, bars, generating, pins, mood } = get();
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
          // An empty box means "pick one for me". Sending "" would be a seed
          // that fails to parse rather than an absent one.
          seed: seed === '' ? null : seed,
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
      // `edited: false` — a fresh generation *is* the seed's own output again,
      // so the project goes back to storing the request rather than the clip.
      set({ pattern, seed: pattern.seed, generating: false, edited: false });
    } catch (error) {
      set({ generating: false, error: reason(error) });
    }
  },

  editPattern(next) {
    // Reference-compared rather than deep-compared: every edit in
    // `PianoRoll/notes.ts` returns the *same* object when it changes nothing
    // (a move clamped to zero, a delete with an empty selection), so this
    // filters those out for free and keeps a no-op gesture off the undo stack.
    if (get().pattern === next) return;
    // ⛔ **`edited` latches here and nowhere else.** From this call on the seed
    // no longer describes what is on screen, so the project has to store the
    // clip rather than the request that made it — see `edited`'s own note.
    set({ pattern: next, edited: true });

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

useSession.subscribe((state, prev) => {
  if (state.pattern === prev.pattern || !isPlugin()) return;
  if (state.pattern !== null) {
    void invoke('arm_pattern', { pattern: state.pattern }).catch(() => {});
  }
  if (state.edited) persist();
});

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
