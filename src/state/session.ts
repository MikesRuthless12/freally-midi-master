import { create } from 'zustand';

import { invoke, isTauri } from '../lib/ipc';
import { isPlugin } from '../lib/ipc-plugin';
import { loadRoster } from '../lib/roster';
import type {
  DatasetProblem,
  Pattern,
  RosterEntry,
  Scale,
  SessionDefaults,
} from '../lib/ipc-types';
import type { DeviceNotice, PlaybackStarted, Playhead } from '../lib/ipc-audio-types';
import { useHistory, type Snapshot } from './history';

/**
 * The one loop the product is about: pick someone, generate, hear it, drag it
 * out (PRD § 1, US-001).
 *
 * One store rather than three, because these things are not independent:
 * generating replaces the pattern, which is what playback plays and what the
 * grid draws, and selecting someone else invalidates all of it. Splitting them
 * would mean keeping three stores in step by hand.
 */

/**
 * The device conditions the transport can report (FR-014).
 *
 * Exported so `locales.test.ts` requires a `device.*` string for each: a state
 * with no catalog entry would render its own key into the transport bar.
 */
export const DEVICE_STATES = ['recovering', 'failed', 'recovered'] as const;

/** What `host_session` reports from the DAW the plugin is loaded in. */
type HostSessionInfo = {
  tempo: number | null;
  timeSigNum: number;
  timeSigDen: number;
  playing: boolean;
};

export type DeviceStateLabel = (typeof DEVICE_STATES)[number];

/** Bar counts the UI offers. Four is the default a pattern is demonstrated at. */
export const BAR_CHOICES = [2, 4, 8] as const;

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
};

export const NO_PINS: SessionPins = { bpm: null, keyRoot: null, scale: null, swing: null };

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
  /** Why playback is unavailable on this machine, if it is. */
  playbackFailure: string | null;
  /**
   * What the output device is doing, when it is not simply working.
   *
   * `null` in the ordinary case. Driven by the `playback:device` event, so an
   * interface pulled out mid-session is visible rather than leaving the app
   * quietly deaf (FR-014).
   */
  deviceState: DeviceStateLabel | null;
  /** Notes the preview kit had no pad for, from the last play. */
  unplacedNotes: number;

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
  generate: () => Promise<void>;
  play: () => Promise<void>;
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
  const { selectedId, seed, bars, pins, autoSync, pattern } = state;
  return { selectedId, seed, bars, pins, autoSync, pattern };
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

  applying = true;
  try {
    set(snapshot);
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
function reason(error: unknown): string {
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
   * Whether the tempo follows the host (TASK-P15).
   *
   * Optional on the way in because a project saved before it existed does not
   * carry it, and absent must mean **on** — the plugin's own
   * `auto_sync_default` makes the same choice for the same reason.
   */
  autoSync?: boolean;
};

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

function send(): void {
  const { selectedId, seed, bars, pins, autoSync } = useSession.getState();
  void invoke('save_session_state', {
    session: { selectedId, seed, bars, pins, autoSync },
  }).catch(() => {
    // Losing a session write is not worth interrupting someone mid-beat. The
    // next change writes the whole session again anyway.
  });
}

function persist(): void {
  // Plugin only. Tauri has its own settings store and a browser has nowhere to
  // put this, and in both the command does not exist — calling it would be a
  // rejected promise per keystroke.
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
  set({
    seed: saved.seed ?? '',
    bars: saved.bars ?? get().bars,
    // Absent means on, matching the plugin's `auto_sync_default`: a project
    // written before the toggle existed must keep following its DAW.
    autoSync: saved.autoSync ?? true,
    pins: {
      bpm: saved.pins?.bpm ?? null,
      keyRoot: saved.pins?.keyRoot ?? null,
      scale: saved.pins?.scale ?? null,
      swing: saved.pins?.swing ?? null,
    },
  });

  // Set directly rather than through `select`, which would clear the pins as a
  // different artist's and raise the keep-or-adopt prompt. This is a session
  // arriving whole, not a switch.
  if (saved.selectedId) {
    set({ selectedId: saved.selectedId });
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
  deviceState: null,
  unplacedNotes: 0,

  pins: NO_PINS,
  hostTempo: null,
  autoSync: true,
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

    // Whether this machine can play at all. Asked once at startup so the
    // transport can be honestly disabled rather than failing on click.
    try {
      const failure = await invoke<string | null>('playback_status');
      set({ playbackFailure: failure });
    } catch {
      // An app with no audio commands registered at all is a dev-mode browser
      // session; the transport stays disabled and nothing is claimed.
      set({ playbackFailure: null });
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
      pattern: null,
      error: null,
      unplacedNotes: 0,
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

  setAutoSync(on) {
    // Saved immediately rather than on the next generation: it is part of how a
    // song was made, and a producer who turns it off and closes the project
    // expects it off when they come back.
    set({ autoSync: on });
    persist();
  },

  async refreshHost() {
    try {
      const host = await invoke<HostSessionInfo>('host_session');
      // A tempo the host has not reported yet arrives as null, and that is a
      // different thing from 0 — the chip must fall back to the artist's value
      // rather than showing a tempo nothing is running at.
      const tempo = typeof host?.tempo === 'number' && host.tempo > 0 ? host.tempo : null;
      if (get().hostTempo !== tempo) set({ hostTempo: tempo });
    } catch {
      // No host behind this UI — the desktop shell, a browser, or a bridge
      // that has no such command. Not an error: there is simply no project
      // tempo to follow, and the artist's value stands.
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

  async generate() {
    const { selectedId, seed, bars, generating, pins } = get();
    if (!selectedId || generating) return;

    set({ generating: true, error: null });
    try {
      const pattern = await invoke<Pattern>('generate_pattern', {
        request: {
          styleId: selectedId,
          bars,
          // An empty box means "pick one for me". Sending "" would be a seed
          // that fails to parse rather than an absent one.
          seed: seed === '' ? null : seed,
          // Every unpinned field goes as null, which serde reads as absent —
          // the artist's own value then stands (FR-002).
          session: pins,
        },
      });
      // Show the seed that was actually used, so the chip can be copied even
      // when the user never typed one (US-004).
      set({ pattern, seed: pattern.seed, generating: false });
    } catch (error) {
      set({ generating: false, error: reason(error) });
    }
  },

  async play() {
    const { pattern } = get();
    if (!pattern) return;
    try {
      const started = await invoke<PlaybackStarted>('play_pattern', {
        pattern,
        looping: true,
      });
      set({ playing: true, unplacedNotes: started.unplacedNotes, error: null });
    } catch (error) {
      set({ playing: false, error: reason(error) });
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
    if (
      state.selectedId === prev.selectedId &&
      state.seed === prev.seed &&
      state.bars === prev.bars &&
      state.pins === prev.pins
    ) {
      return;
    }
    persist();
  });

  // The page is going away — `pagehide` is the last event a webview reliably
  // delivers, and `visibilitychange` covers a host that hides the editor
  // without destroying it. Both are cheap no-ops when nothing is pending.
  window.addEventListener('pagehide', flush);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flush();
  });
}

/**
 * Follow the playhead the audio thread publishes.
 *
 * Only inside Tauri: in a browser there is no event system behind it, and the
 * transport state is driven by the play/stop calls alone.
 */
export async function subscribeToPlayhead(): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import('@tauri-apps/api/event');

  const playhead = await listen<Playhead>('playback:playhead', (event) => {
    useSession.setState({
      playing: event.payload.playing,
      playhead: event.payload.position,
    });
  });

  // The device coming and going (FR-014). A recovery clears itself after a few
  // seconds — it is news, not a condition — while "lost" and "failed" stay up
  // for as long as they are true.
  let clearRecovered: number | undefined;
  const device = await listen<DeviceNotice>('playback:device', (event) => {
    window.clearTimeout(clearRecovered);
    const { state, recovered } = event.payload;

    if (recovered) {
      useSession.setState({ deviceState: 'recovered' });
      clearRecovered = window.setTimeout(
        () => useSession.setState({ deviceState: null }),
        6_000,
      );
      return;
    }

    useSession.setState({
      deviceState: state === 'failed' ? 'failed' : 'recovering',
      // The device is gone, so nothing is playing, whatever the last playhead
      // event happened to say.
      playing: false,
    });
  });

  return () => {
    window.clearTimeout(clearRecovered);
    playhead();
    device();
  };
}
