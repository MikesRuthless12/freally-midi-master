import { create } from 'zustand';

import { invoke } from '../lib/ipc';
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
  /**
   * The genre this take was generated OVER, or `null` for the artist's own
   * (TASK-158C).
   *
   * ⛔ **Recorded for the same reason `mood` is**: recall regenerates from the
   * inputs rather than replaying stored notes, so an input the entry does not
   * carry is one the recall silently takes from whatever is pinned *now*. A take
   * made over boom-bap, recalled after the chip moved, would come back as notes
   * that are not the take the panel is describing.
   *
   * ⚠ Optional on the way in: entries written before this existed have none, and
   * absent means the artist's own — which is what they were.
   */
  base?: string | null;
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

/**
 * The key a kept take is remembered by.
 *
 * ⛔ Part **and** seed, because a seed is only unique within a part: the drums
 * and the melody of one record share a song seed, and keeping the melody must
 * not silently keep the drums with it.
 */
export const keptKey = (part: Part, seed: string) => `${part}:${seed}`;

/**
 * A `.mid` from the browser, kept to train on (TASK-040T).
 *
 * ⚠ **The patterns are the file's own split**, one per part, exactly as
 * `explorer_midi_split` answered — the same `Pattern` shape a kept generation is
 * regenerated into, so `engine::fit` measures a file and a take through one path
 * rather than two.
 */
export type KeptFile = { path: string; patterns: Pattern[] };

type VariationsState = {
  /** Every generation, per part, oldest first. */
  entries: Record<string, Variation[]>;
  /**
   * The takes the producer marked to train on (TASK-040T).
   *
   * ⛔ **A set of keys rather than a flag on `Variation`.** Keeping is an
   * opinion about a take, and the log is a record of what happened — stepping
   * back through history must not be able to change what was kept, and a flag
   * living inside the entry is one careless `record` away from doing exactly
   * that.
   */
  kept: Record<string, true>;
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
  /**
   * Park on a specific take of this session's log, by seed (TASK-045B).
   *
   * ⛔ **Because the browsable panel does not step, it jumps.** `step` moves by a
   * delta and is what ◀/▶ use; choosing take twelve out of eighty is not a delta.
   * Without this the panel recalled the take and left the cursor where it was, so
   * the counter went on saying "80 / 80" over take twelve and the very next ◀
   * jumped back to 79 — the readout and the clip disagreeing about which take the
   * producer is on.
   *
   * ⚠ **A no-op for a take from a previous session**, which is not a failure: the
   * persisted history spans restarts and this session's log does not, so a take
   * older than the page has no position to park on. It is still recalled — the
   * panel does that — and the counter honestly goes on describing this session.
   */
  parkOn: (part: Part, seed: string) => void;
  /** Mark a take to train on, or unmark it. */
  keep: (part: Part, seed: string, kept: boolean) => void;
  /** Every kept take, oldest first, across every part. */
  keptEntries: () => Variation[];
  /**
   * Files the producer has marked to train on (TASK-040T), by path.
   *
   * ⛔⛔ **These carry their NOTES, and everything else in this store carries a
   * seed.** The file's own header says the notes are never stored because the
   * engine is deterministic and an entry regenerates them exactly — that
   * argument holds for a generation and is simply false for somebody else's
   * `.mid`. Nothing can rebuild a file from a number, so a kept file is the one
   * thing here that has to be the material itself.
   *
   * ⚠ **Keyed by path, so keeping the same file twice keeps it once.** A fit is
   * a measurement of a distribution: the same eight bars counted three times is
   * a producer's taste reported as three times more certain than it is.
   *
   * ⚠ **Not persisted, and that is the same trade `entries` takes.** This is the
   * session's own working set; the paths are on disk and the gesture is one
   * press to redo.
   */
  keptFiles: Record<string, KeptFile>;
  /** Mark a file to train on, or unmark it. */
  keepFile: (file: KeptFile, kept: boolean) => void;
  /** Every pattern kept from a file, in the order the files were kept. */
  keptFilePatterns: () => Pattern[];
  /**
   * Every generation of every previous session, by style id (TASK-045B).
   *
   * ⛔ **Separate from `entries`, and that is the honest shape.** `entries` is
   * *this* session's log, keyed by part, and it is what ◀/▶ walk. This is what
   * the plugin has on disk: it spans restarts, and it is grouped the way Mike
   * described browsing it — *"20 just 'Trap' and 20 just 'Rage' and 40 just
   * 'Drake'"*. Merging them into one structure would mean either losing the
   * per-part cursor or losing the per-style grouping.
   */
  history: Record<string, Variation[]>;
  /** Read the persisted history. Called once when the panel opens. */
  loadHistory: () => Promise<void>;
  /** Forget the persisted history. */
  clearHistory: () => Promise<void>;
  /** Forget everything. Used by tests; nothing in the UI calls it. */
  reset: () => void;
};

/**
 * Takes waiting to be written, and the flush that is already queued.
 *
 * ⛔⛔ **Because one Generate press is FIVE recorded takes, not one.**
 * `session.ts` records an entry per generated part, and `takes::note` does a
 * whole read-parse-serialize-write of `takes.json` per call — a file the module
 * bounds at 3,200 takes. Unbatched, one press was five complete rewrites of it,
 * synchronously on the host's editor thread.
 *
 * ⚠ **A microtask, not a timer.** The five `record` calls happen in one
 * synchronous run, so the queue is drained on the very next tick — nothing waits
 * on a clock, and a producer who generates and immediately closes the window has
 * already had their takes sent.
 */
let pending: Variation[] = [];

function keepLater(entry: Variation) {
  pending.push(entry);
  if (pending.length > 1) return;
  queueMicrotask(() => {
    const takes = pending;
    pending = [];
    // ⚠ Failure is swallowed, the trade `recent::note` records on the other
    // history: a producer whose `%APPDATA%` is read-only should still get their
    // beat. Refusing the generation to protect the bookkeeping is the wrong way
    // round.
    void invoke('takes_note', { takes }).catch(() => {});
  });
}

export const useVariations = create<VariationsState>((set, get) => ({
  entries: {},
  position: {},
  kept: {},
  keptFiles: {},
  history: {},

  record(entry) {
    // ⛔⛔ **Written to disk as well as to the log** (TASK-045B). Mike: *"if you
    // have generated 20 just 'Trap' and 20 just 'Rage' and 40 just 'Drake' then
    // it should persist … so that way you can go through the actual history of
    // all your generations and find what you like."*
    //
    // ⚠ **Fire-and-forget, and the failure is swallowed** — the same trade
    // `recent::note` records on the other history: a producer whose `%APPDATA%`
    // is read-only should still get their beat. Refusing the generation to
    // protect the bookkeeping would be the wrong way round.
    //
    // ⚠ **Here rather than in `generate`**, because this is the one function
    // every path to a recorded take goes through — and `session.ts`'s
    // `recalling` guard already keeps a *recall* from reaching it, which is what
    // stops stepping backwards from writing new history.
    //
    // ⚠ **The reply is an acknowledgement, not the history.** `loadHistory` is
    // what fills `history`, and it runs when the panel that draws it opens —
    // handing the whole map back here would serialize up to 3,200 takes across
    // the bridge on every press, for a list nothing is showing.
    keepLater(entry);
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

  parkOn(part, seed) {
    const { entries, position } = get();
    const at = (entries[part] ?? []).findIndex((entry) => entry.seed === seed);
    if (at < 0) return;
    set({ position: { ...position, [part]: at } });
  },

  keep(part, seed, kept) {
    // ⚠ **Removed rather than set to `false`.** The field's own doc calls this a
    // set of keys, and storing `false` forever made every reader spell
    // `=== true` to avoid trusting it — a shape that invites the one caller who
    // forgets.
    set((state) => {
      const next = { ...state.kept };
      if (kept) {
        next[keptKey(part, seed)] = true;
      } else {
        delete next[keptKey(part, seed)];
      }
      return { kept: next };
    });
  },

  keptEntries() {
    const { entries, kept } = get();
    return Object.values(entries)
      .flat()
      .filter((entry) => kept[keptKey(entry.part, entry.seed)]);
  },

  keepFile(file, kept) {
    // Removed rather than set to a falsy value, for the reason `keep` above
    // gives at length.
    set((state) => {
      const next = { ...state.keptFiles };
      if (kept) {
        next[file.path] = file;
      } else {
        delete next[file.path];
      }
      return { keptFiles: next };
    });
  },

  keptFilePatterns() {
    return Object.values(get().keptFiles).flatMap((file) => file.patterns);
  },

  async loadHistory() {
    try {
      set({ history: await invoke<Record<string, Variation[]>>('takes_list') });
    } catch {
      // No history is not a broken app — the same reading `loadRecent` takes.
      // The panel says "nothing yet", which is true of a first run either way.
    }
  },

  async clearHistory() {
    try {
      set({ history: await invoke<Record<string, Variation[]>>('takes_clear') });
    } catch {
      // ⚠ Silent for the same reason, and the panel still shows what it has:
      // reporting a failed clear as an empty list would be the readout lying
      // about a file that is still on disk.
    }
  },

  reset() {
    set({ entries: {}, position: {}, kept: {}, keptFiles: {}, history: {} });
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
  session: { mood: string | null; base: string | null; pins: SessionPins },
  at: number,
): Variation {
  return {
    part: pattern.part,
    artistId: pattern.artistId,
    mood: session.mood,
    base: session.base,
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
