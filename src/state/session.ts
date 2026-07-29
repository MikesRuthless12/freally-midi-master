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
  /** Ask the host what tempo it is running at. No-op outside a plugin. */
  refreshHost: () => Promise<void>;
  /** Keep the pinned session over the new artist's defaults. */
  keepPins: () => void;
  /** Drop every pin and let the new artist decide. */
  adoptDefaults: () => void;
  generate: () => Promise<void>;
  play: () => Promise<void>;
  stop: () => Promise<void>;
};

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
type SavedSession = {
  selectedId: string | null;
  seed: string;
  bars: number | null;
  pins: Partial<SessionPins> | null;
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

function persist(get: () => SessionState): void {
  // Plugin only. Tauri has its own settings store and a browser has nowhere to
  // put this, and in both the command does not exist — calling it would be a
  // rejected promise per keystroke.
  if (!isPlugin()) return;

  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    const { selectedId, seed, bars, pins } = get();
    void invoke('save_session_state', {
      session: { selectedId, seed, bars, pins },
    }).catch(() => {
      // Losing a session write is not worth interrupting someone mid-beat.
      // The next change writes the whole session again anyway.
    });
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
async function restore(
  set: (partial: Partial<SessionState>) => void,
  get: () => SessionState,
): Promise<void> {
  if (!isPlugin()) return;

  let saved: SavedSession;
  try {
    saved = await invoke<SavedSession>('session_state');
  } catch {
    return;
  }
  if (!saved) return;

  // Field by field rather than spread: the plugin's pins are the engine's
  // six-field `SessionOverrides` and this store's are four, so a spread would
  // put `bars` and `halfTime` into a shape that has no room for them.
  set({
    seed: saved.seed ?? '',
    bars: saved.bars ?? get().bars,
    pins: {
      bpm: saved.pins?.bpm ?? null,
      keyRoot: saved.pins?.keyRoot ?? null,
      scale: saved.pins?.scale ?? null,
      swing: saved.pins?.swing ?? null,
    },
  });

  // Set directly rather than through `select`, which would clear the pins as a
  // different artist's and raise the keep-or-adopt prompt. This is the *same*
  // session coming back, not a switch.
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
  defaults: null,
  pendingArtist: null,

  async init() {
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

    // After the roster, because restoring a selection wants the entry to exist
    // for the rail to highlight, and because `loadDefaults` reads the dataset.
    await restore(set, get);
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
    persist(get);
  },

  setSeed(seed) {
    set({ seed: seed.trim() });
    persist(get);
  },

  setBars(bars) {
    set({ bars });
    persist(get);
  },

  setPin(field, value) {
    set({ pins: { ...get().pins, [field]: value } });
    persist(get);
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

  keepPins() {
    set({ pendingArtist: null });
    persist(get);
  },

  adoptDefaults() {
    set({ pins: NO_PINS, pendingArtist: null });
    persist(get);
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
      // The seed the engine chose is the one that reproduces this beat, so it
      // is the one worth saving — an unsaved fresh seed would reopen the
      // project on a different pattern by the same artist.
      persist(get);
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
}));

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
