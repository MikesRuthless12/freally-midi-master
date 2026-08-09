import { create } from 'zustand';

import type { Part, Pattern, Scale } from '../lib/ipc-types';
import type { SessionPins } from './session';

/**
 * The variation history (TASK-045, as Mike rescoped it 2026-07-29).
 *
 * ⛔ **Every generation of the session, from the first — not the last 20.**
 * Mike: *"keep going sequentially through the seeds that you have generated
 * since the beginning of the app."* The cap was there to bound memory and it
 * does not need to be: an entry is a seed, an artist id, a mood, bars and the
 * pins — tens of bytes. Ten thousand generations is a rounding error next to
 * one pattern's notes, and **the notes are not stored**, because the engine is
 * deterministic and the entry regenerates them exactly. That is the same
 * argument `plugin/src/state.rs` already makes about the project file.
 *
 * ⛔ **Stepping restores the whole setup, not just the number.** Mike: *"it
 * should automatically select which artist/workflow was selected for that
 * actual seed and the genre/mood for that seed, so that way you know exactly
 * how you got each generation."* A recall that restored the seed alone would
 * regenerate a *different* beat whenever the artist had changed since — which
 * is the readout lying about how you got there, and worse than no history.
 *
 * ⛔⛔ **The entry records the tempo and key that were actually *used*, not just
 * what was asked for.** Mike: *"it should also show the scale and bpm that was
 * used for that seed's particular generation, even if it differs from the
 * auto-sync."* Two reasons, and the second is the important one:
 *
 * 1. A generation made while the DAW sat at 92 was made at 92, and the pins may
 *    say nothing at all about that. Showing the pins would show blank.
 * 2. **Tempo changes the notes.** `SessionContext.bpm` reaches `humanize`
 *    through `ms_to_ticks`, so the same seed at a different tempo is a
 *    different pattern. An entry that recorded only the seed would not reproduce
 *    its own beat once the project's tempo had moved.
 *
 * ⛔ **This is not the undo stack, and the two must not be folded together.**
 * `history.ts` is a document-state stack a producer steps through with Ctrl+Z;
 * this is a log of *generations*, which are one kind of thing that happens to a
 * document. Undo has to take back an edit; this has to take you back to a beat.
 */

/** One generation, and everything needed to get back to it. */
export type Variation = {
  /** Which generator this was, so the counters are per part. */
  part: Part;
  artistId: string;
  /** The pinned mood, or `null` for "Any" (TASK-040V). */
  mood: string | null;
  /** The take's seed, as a decimal string — a `u64` does not survive a number. */
  seed: string;
  /** The record every part of this generation was written against (TASK-141). */
  songSeed: string;
  bars: number;
  /** What was pinned at the time. Absent fields mean the artist decided. */
  pins: SessionPins;
  /**
   * ── What was actually used ──────────────────────────────────────────────
   *
   * ⛔ Resolved, never the pins. See the module note: the pins can be silent
   * about the very tempo that produced these notes.
   */
  bpm: number;
  keyRoot: number;
  scale: Scale;
  timeSigNum: number;
  timeSigDen: number;
  /**
   * When it was made, epoch milliseconds.
   *
   * Mike: *"record the date/time of that specific seed, so it can show the end
   * user you created this seed on this day of this month of this year at this
   * specific time."* Taken here, in the frontend, and **never read by the
   * engine** — so nothing about generation depends on a clock.
   */
  at: number;
};

type VariationsState = {
  /** Every generation, per part, oldest first. */
  entries: Record<string, Variation[]>;
  /** Where the producer is looking, per part. `-1` before the first generation. */
  position: Record<string, number>;
  /** Append a generation and park on it. */
  record: (entry: Variation) => void;
  /**
   * Move `delta` entries through one part's history, and hand back where you
   * land — or `null` when there is nowhere to go.
   */
  step: (part: Part, delta: number) => Variation | null;
  /** The entry currently parked on, or `null`. */
  current: (part: Part) => Variation | null;
  /** Forget everything. Used by tests; nothing in the UI calls it. */
  reset: () => void;
};

export const useVariations = create<VariationsState>((set, get) => ({
  entries: {},
  position: {},

  record(entry) {
    set((state) => {
      const list = [...(state.entries[entry.part] ?? []), entry];
      return {
        entries: { ...state.entries, [entry.part]: list },
        // ⛔ **Appended, never truncated, and that is Mike's "starts a new
        // branch from there rather than silently discarding the entries
        // ahead".** Stepping back is *browsing*; generating from there adds to
        // the end and parks you on it. Nothing a producer reached is lost —
        // which is the thing the instruction is actually about, and losing
        // forward history is what would cost someone a beat they liked.
        position: { ...state.position, [entry.part]: list.length - 1 },
      };
    });
  },

  step(part, delta) {
    const { entries, position } = get();
    const list = entries[part] ?? [];
    if (list.length === 0) return null;

    const from = position[part] ?? list.length - 1;
    const to = Math.min(list.length - 1, Math.max(0, from + delta));
    // ⚠ Clamped rather than wrapped. Wrapping at the ends would take a producer
    // stepping back through a thousand generations to the newest one without
    // anything saying so — the opposite of "you know exactly how you got here".
    if (to === from) return null;

    set({ position: { ...position, [part]: to } });
    return list[to];
  },

  current(part) {
    const { entries, position } = get();
    const list = entries[part] ?? [];
    const at = position[part] ?? list.length - 1;
    return list[at] ?? null;
  },

  reset() {
    set({ entries: {}, position: {} });
  },
}));

/** How many generations this part has had, and which one is on screen. */
export function counter(part: Part): { position: number; total: number } {
  const { entries, position } = useVariations.getState();
  const total = (entries[part] ?? []).length;
  // 1-based for the readout, because "1 / 300" is what Mike asked for and
  // nobody counts their first take as zero.
  return { position: total === 0 ? 0 : (position[part] ?? total - 1) + 1, total };
}

/**
 * The entry a pattern and the session that produced it make.
 *
 * ⛔ **The resolved values come off the `Pattern`, not off the store.** The
 * engine answers with the `bpm`, `keyRoot`, `scale` and meter it actually used,
 * and those are the only ones that reproduce this take.
 */
export function entryFor(
  pattern: Pattern,
  session: { mood: string | null; pins: SessionPins },
  at: number,
): Variation {
  return {
    part: pattern.part,
    artistId: pattern.artistId,
    mood: session.mood,
    seed: pattern.seed,
    songSeed: pattern.songSeed,
    bars: pattern.bars,
    pins: session.pins,
    bpm: pattern.bpm,
    keyRoot: pattern.keyRoot,
    scale: pattern.scale,
    timeSigNum: pattern.timeSigNum,
    timeSigDen: pattern.timeSigDen,
    at,
  };
}

/**
 * When a generation was made, written the way Mike asked for it:
 * `Thursday, August 13, 2026 @9:54 PM CST`.
 *
 * ⛔ **`Intl.DateTimeFormat`, not a hand-rolled string.** That renders exactly
 * the above in `en` *and* stays correct in the other 17 locales — which pick
 * their own field order, their own month names and 24-hour time where that is
 * the convention. A literal `dddd, MMMM D, YYYY` would be right in one locale
 * and wrong in most, and this app ships RTL ones.
 *
 * ⚠ **Two formatters joined by `" @"`, because `timeStyle` cannot be combined
 * with `timeZoneName`.** The zone abbreviation is the *viewer's*, which is the
 * right one: it answers "when did I make this", not "what was UTC".
 */
export function madeAt(at: number, locale: string): string {
  if (at <= 0) return '';
  const { day, time } = formatters(locale);
  return `${day.format(at)} @${time.format(at)}`;
}

/**
 * The two formatters, built once per locale.
 *
 * ⛔ **Constructing an `Intl.DateTimeFormat` is not free, and this is called
 * from a component that re-renders with the playhead.** `VariationNav` fills a
 * `title` from it, `CenterStage` subscribes to `playhead`, and the playhead is
 * published at 30 Hz — so the naive form built *sixty* formatters a second, for
 * a string that is only visible on hover.
 */
const FORMATTERS = new Map<string, { day: Intl.DateTimeFormat; time: Intl.DateTimeFormat }>();

function formatters(locale: string) {
  const cached = FORMATTERS.get(locale);
  if (cached !== undefined) return cached;
  const built = {
    day: new Intl.DateTimeFormat(locale, { dateStyle: 'full' }),
    time: new Intl.DateTimeFormat(locale, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }),
  };
  FORMATTERS.set(locale, built);
  return built;
}
