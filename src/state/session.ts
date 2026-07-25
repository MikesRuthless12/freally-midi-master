import { create } from 'zustand';

import { invoke, isTauri } from '../lib/ipc';
import { loadRoster } from '../lib/roster';
import type { DatasetProblem, Pattern, RosterEntry } from '../lib/ipc-types';
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

export type DeviceStateLabel = (typeof DEVICE_STATES)[number];

/** Bar counts the UI offers. Four is the default a pattern is demonstrated at. */
export const BAR_CHOICES = [2, 4, 8] as const;

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

  init: () => Promise<void>;
  select: (id: string) => void;
  setSeed: (seed: string) => void;
  setBars: (bars: number) => void;
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
  },

  select(id) {
    if (get().selectedId === id) return;
    // The old pattern belongs to the old artist. Keeping it on screen under a
    // new name would be the most convincing wrong thing the app could show.
    set({ selectedId: id, pattern: null, error: null, unplacedNotes: 0 });
  },

  setSeed(seed) {
    set({ seed: seed.trim() });
  },

  setBars(bars) {
    set({ bars });
  },

  async generate() {
    const { selectedId, seed, bars, generating } = get();
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
