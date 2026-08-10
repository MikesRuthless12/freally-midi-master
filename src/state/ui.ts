import { create } from 'zustand';
import { readStored, writeStored } from './storage';
import { applyLanguage, loadLanguagePreference } from '../i18n';
import { type LocaleCode } from '../i18n/locales';
import { applyThemePreference, loadThemePreference, type ThemePreference } from './theme';
import type { Lane, Part } from '../lib/ipc-types';
// ⚠ `state/lanes.ts` is a leaf that imports nothing but a type, which is why it
// can be reached from here without dragging the kit store — and its own doc
// records what happened the last time that list lived somewhere less reachable.
import { ALL_LANES } from './lanes';

/** The six generators. Order matches the tab strip in PRD § 8. */
export const GENERATOR_TABS = ['drums', 'melody', 'counter', 'bass', 'chords', 'song'] as const;

export type GeneratorTab = (typeof GENERATOR_TABS)[number];

/** Below this the right rail collapses (PRD § 8). */
export const WIDE_BREAKPOINT = 1440;

/** Individually collapsible panels. The right rail as a whole is separate — it
 *  is driven by the viewport breakpoint and the K shortcut. */
export const SECTIONS = [
  'genres',
  'roster',
  'explorer',
  'kit',
  'stems',
  'session',
  'presets',
  'patterns',
] as const;
export type SectionId = (typeof SECTIONS)[number];

export type SectionState = Record<SectionId, boolean>;

const SECTIONS_KEY = 'freally.sections';

const REDUCE_MOTION_KEY = 'freally.reduceMotion';
const PADS_KEY = 'freally.pads';

/**
 * How many pads may address the same lane.
 *
 * ⛔ **Two, so a snare can be layered** (Mike, 2026-08-09). One would forbid the
 * thing he asked for; unlimited would let a producer fill all eight with the same
 * lane and lose the kit with no way to see what happened.
 */
export const PAD_LIMIT = 2;

/**
 * Which eight lanes the stage pads address, in order.
 *
 * ⛔ **Eight, and every one of them swappable** — Mike, 2026-08-09: *"ensure that
 * the names of the eight lanes are interchangeable with comboboxes so an end
 * user can switch them out for other lane names."* The default is what a trap,
 * drill or boom-bap beat is built from; a producer working in anything else
 * changes them, and the choice sticks.
 *
 * ⚠ The other twenty-nine lanes never went anywhere — the KIT panel in the right
 * rail still lists all of them. These eight are the shortcut.
 */
export const DEFAULT_PADS = [
  'kick',
  'snare',
  'clap',
  'closedHat',
  'openHat',
  'perc',
  'rim',
  'crash',
] as const;

/**
 * ⛔ **Validated against the real lane list on the way in, not trusted.** This is
 * localStorage: it survives updates, and a lane renamed or removed by a later
 * release would otherwise leave a pad addressing something that no longer
 * exists — a pad that silently does nothing, which is the worst kind. Anything
 * unrecognised falls back to the default for that slot.
 */
function cleanPads(parsed: unknown): string[] {
  const fallback = [...DEFAULT_PADS];
  if (!Array.isArray(parsed)) return fallback;
  return fallback.map((lane, at) =>
    typeof parsed[at] === 'string' && ALL_LANES.includes(parsed[at] as Lane)
      ? (parsed[at] as string)
      : lane,
  );
}

/**
 * The pad layout **per style**, keyed by style id.
 *
 * ⛔⛔ **One shared layout was wrong, and Mike caught it immediately.**
 * 2026-08-09: *"when i click to open my 'My EDM' it should go back to having a
 * kick for the first drum lane instead of a 'Sub Kick' that i changed it to when
 * i switched the artist — the original workflows should go back to exactly how
 * they were when you left them."* A single list meant reaching for a sub kick
 * while working on a drill beat silently rewrote the layout of every other style
 * the producer owns, and there was nothing on screen to say it had happened.
 *
 * ⚠ Which lanes are on the pads is a property of **the thing being made**, like
 * the kit itself — not of the window it is made in.
 */
function loadPads(): Record<string, string[]> {
  try {
    const raw = window.localStorage.getItem(PADS_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {};
    const out: Record<string, string[]> = {};
    for (const [id, lanes] of Object.entries(parsed as Record<string, unknown>)) {
      out[id] = cleanPads(lanes);
    }
    return out;
  } catch {
    return {};
  }
}

/**
 * Whether the user asked for less animation.
 *
 * ⛔ In localStorage, like the theme and the language. It used to live only in
 * `settings.json`, which the desktop shell owned — so when that shell was
 * removed the preference had nowhere left to go and reset on every launch. Off
 * by default, so the OS setting decides unless someone says otherwise here.
 */
function loadReduceMotion(): boolean {
  return readStored(REDUCE_MOTION_KEY, (v): v is string => v === 'true', 'false') === 'true';
}

const ALL_OPEN: SectionState = {
  genres: true,
  roster: true,
  explorer: true,
  kit: true,
  stems: true,
  session: true,
  presets: true,
  patterns: true,
};

function loadSections(): SectionState {
  try {
    const raw = window.localStorage.getItem(SECTIONS_KEY);
    if (!raw) return ALL_OPEN;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) return ALL_OPEN;
    // Merge over the defaults so a section added in a later version defaults to
    // visible rather than vanishing for anyone with an older stored value.
    const stored = parsed as Partial<Record<SectionId, unknown>>;
    const out = { ...ALL_OPEN };
    for (const id of SECTIONS) {
      if (typeof stored[id] === 'boolean') out[id] = stored[id];
    }
    return out;
  } catch {
    return ALL_OPEN;
  }
}

function saveSections(sections: SectionState): void {
  writeStored(SECTIONS_KEY, JSON.stringify(sections));
}

type UiState = {
  activeTab: GeneratorTab;
  /** Whether the right rail is showing. Follows the breakpoint until the user
   *  overrides it with K, which is why it is stored rather than derived. */
  rightRailOpen: boolean;
  sections: SectionState;
  /** Every style's pad layout, by style id. Read it through `padsFor`. */
  pads: Record<string, string[]>;
  /** The eight lanes this style's pads address, defaulted when it has none yet. */
  padsFor: (styleId: string | null) => string[];
  /** Put a different lane on one of this style's pads. Persisted immediately. */
  setPad: (styleId: string | null, at: number, lane: string) => void;
  theme: ThemePreference;
  /**
   * The Settings toggle that suppresses the generation animation.
   *
   * Distinct from the OS's `prefers-reduced-motion`, which is read where it is
   * used. This can only ever turn motion *off*: on means off, off means "ask
   * the OS" (FR-017).
   */
  reduceMotion: boolean;
  language: LocaleCode;
  /**
   * Whether this session has already shown the Stems panel (TASK-063C).
   *
   * ⛔ **Store state rather than a module flag, and not persisted.** Not
   * persisted because the rule is "the first generation of a *session*" — a
   * producer who collapsed it yesterday still needs to be shown it once today,
   * which is the whole complaint. In the store rather than a `let` beside the
   * subscriber because a hidden module variable cannot be reset between tests,
   * and a one-shot nothing can re-arm is a one-shot nothing can prove.
   */
  stemsRevealed: boolean;
  /**
   * Generators switched OFF for playback (TASK-127).
   *
   * ⛔⛔ **Mike, 2026-08-06:** *"i want to be able to play the generators all at
   * once or separately, they should be able to be toggled on and off for each
   * generator."* Play sounds every generated part that is not in here, merged
   * into the one clip a schedule can hold.
   *
   * ⚠ **OFF is what is stored, so ON is the default and stays the default.**
   * Holding the on-set instead would mean a part generated after the toggles
   * were last touched arrives silent, with nothing on screen saying why — a
   * producer would press Generate on the bassline and hear nothing.
   *
   * ⚠ Not persisted. It is an audition choice about this sitting, like solo on a
   * mixer, rather than a property of the record.
   */
  partsOff: Part[];
  /**
   * Whether the clip repeats at its end (TASK-138).
   *
   * ⛔ Mike, 2026-08-06: *"can you have the 'Loop' button toggle off and on and
   * either loop every time it plays to the end of the 4 or 8 bars or stop at the
   * end of the 4 or 8 bars."* On by default, which is what the button has always
   * claimed — its tooltip read *"Playback always loops in this phase."*
   *
   * ⚠ **The plugin holds the authority**, on `Shared`, where the audio thread
   * reads it every block. This is the page's copy for drawing the button, and
   * `transport_loop` is what keeps them in step.
   */
  looping: boolean;

  setActiveTab: (tab: GeneratorTab) => void;
  /** Switch one generator's playback on or off (TASK-127). */
  togglePart: (part: Part) => void;
  /** Turn the loop on or off (TASK-138). */
  toggleLooping: () => void;
  /**
   * Set it outright, for hydrating from the plugin on mount.
   *
   * ⚠ A toggle cannot hydrate: it flips whatever the page happened to default
   * to, so restoring a known value needs to state it. The plugin outlives the
   * webview and Loop is its state, not the page.
   */
  setLooping: (on: boolean) => void;
  toggleRightRail: () => void;
  /** Called when the viewport crosses WIDE_BREAKPOINT. */
  setWide: (wide: boolean) => void;
  toggleSection: (id: SectionId) => void;
  /**
   * Put the Stems panel in front of someone who has just generated something.
   *
   * ⛔⛔ **Mike, 2026-08-06:** *"the stems panel should be visible if you have
   * done a generation so that way you can ensure that you can drag it in no
   * matter what right away."* Panels remember their collapsed state across
   * reloads, so a producer who once collapsed this one would never see the drag
   * rows again — and nothing anywhere else in the UI says they exist. The panel
   * holds the only way to get a pattern out of the plugin.
   *
   * ⛔⛔ **The section only — this must NOT force the right rail open, and the
   * first cut of it did.** `e2e/piano-roll.spec.ts:380` caught it: opening the
   * rail re-lays the stage, the velocity lane loses height, and a drag to
   * velocity 96 landed on 85. That is the third time this project has been
   * bitten by growing something near the pattern, and `StemsPanel`'s own header
   * records the other two.
   *
   * ⚠ **And it buys nothing in the plugin, which is where the complaint came
   * from.** The page always lays out at `LAYOUT` (1440) whatever size the window
   * is drawn at — that is the whole point of the editor's scaling — and
   * `WIDE_BREAKPOINT` is the same 1440, so `isWide()` is true and the rail is
   * already open. Forcing it would only ever have affected a narrow browser
   * window, at the cost of the editor above it.
   *
   * Idempotent, and that is load-bearing: it is called on every write that
   * leaves a pattern in the store, so a producer who closes the panel after it
   * has been shown must not have it reopened under them on the next Generate.
   */
  revealStems: () => void;
  setAllSections: (open: boolean) => void;
  setTheme: (theme: ThemePreference) => void;
  setReduceMotion: (reduce: boolean) => void;
  setLanguage: (language: LocaleCode) => void;
};

/**
 * Whether the app has the width the right rail needs — measured on the layout,
 * **not on the viewport**.
 *
 * ⛔ `window.innerWidth` and `matchMedia` are both wrong here, and wrong in a
 * way that is invisible until the plugin is on a real screen. The plugin scales
 * its window and applies a matching CSS `zoom` to the root, so the page lays out
 * at the full 1440 inside a smaller window — but zoom changes neither the
 * viewport nor media-query evaluation. Reading either of them makes the rail
 * collapse at *every* scale below 1.0, which is the exact failure the scaling
 * was written to prevent.
 *
 * `documentElement.clientWidth` is reported in the root's own (zoomed)
 * coordinates, so it is the number the layout actually gets. Measured: in a
 * 1224px viewport with `zoom: 0.85`, `innerWidth` stays 1224 and the 1440 media
 * query stays false, while this returns 1440.
 */
/**
 * ⛔ **The slack is not a fudge — the layout width IS the breakpoint.**
 *
 * `LAYOUT` in `plugin/src/editor.rs` is 1440 wide and [`WIDE_BREAKPOINT`] is
 * 1440, so the rail sits exactly on the edge at every scale, with nothing to
 * spare. The width then arrives through a floating-point round trip: the window
 * is `1440 * factor` **rounded to whole pixels**, and the page divides by that
 * same factor to get back. At `factor: 1.0` that is exact; at anything else it
 * can land a pixel low — `1224 / 0.85` is `1439.999…` — and one pixel was the
 * difference between the rail being there and not.
 *
 * ▶ **Mike, 2026-08-06:** *"how come the smaller default size doesn't show the
 * stems panel when it is supposed to be shown, but the bigger size does."* That
 * is this: the larger preset zoomed by 1.0 and kept the rail, every other preset
 * lost it.
 *
 * ⚠ Wide enough to also absorb a scrollbar gutter, which `clientWidth` excludes
 * and which is the other way this measurement comes up short of the layout the
 * page was given. Still far narrower than any real step down — the next honest
 * reason to collapse the rail is a host window hundreds of pixels smaller.
 */
const BREAKPOINT_SLACK = 24;

export function isWide(): boolean {
  if (typeof document === 'undefined') return true;
  const layout = document.documentElement.clientWidth || window.innerWidth;
  return layout >= WIDE_BREAKPOINT - BREAKPOINT_SLACK;
}

const startsWide = typeof window === 'undefined' ? true : isWide();

/**
 * The last answer the breakpoint gave, so repeating it cannot undo a K toggle.
 *
 * ⛔⛔ **The crossing check belongs here, because more than one caller has it to
 * get right and one of them did not.** `setWide` used to be an unconditional
 * `set({ rightRailOpen: wide })`, and `App.tsx`'s resize listener guarded it by
 * hand — a guard `WindowSize.tsx::applyZoom` did not have. Once `applyZoom`
 * started running on every `resize` (so the zoom could be re-derived when a
 * queued editor resize finally landed), every resize wrote the breakpoint's
 * answer back over the producer's own choice: press K to collapse the rail, let
 * the host resize the window, and the rail snapped open again — re-laying the
 * stage and taking height off the velocity lane, which is the exact regression
 * `e2e/piano-roll.spec.ts:380` exists to catch. In the plugin it is guaranteed,
 * not occasional: the page always lays out at `LAYOUT` 1440 and
 * `WIDE_BREAKPOINT` is also 1440, so `isWide()` is *always* true there.
 *
 * ⚠ **Module-level rather than store state**, because nothing renders from it
 * and there is exactly one window per page. `toggleRightRail` deliberately does
 * not touch it: a manual toggle is not the breakpoint changing its mind, and
 * recording it as one would let the next resize "restore" the wrong thing.
 */
let lastBreakpoint = startsWide;

export const useUi = create<UiState>((set, get) => ({
  activeTab: 'drums',
  rightRailOpen: startsWide,
  sections: loadSections(),
  pads: loadPads(),

  padsFor: (styleId) =>
    styleId === null ? [...DEFAULT_PADS] : (get().pads[styleId] ?? [...DEFAULT_PADS]),

  setPad: (styleId, at, lane) =>
    set((s) => {
      // Nothing selected is nothing to remember it against — and there is no
      // pad grid to change either, so this cannot be reached in practice.
      if (styleId === null) return s;
      const current = s.pads[styleId] ?? [...DEFAULT_PADS];

      // ⛔ **At most two pads on one lane** — Mike, 2026-08-09: *"only 2
      // instruments can be the same, so that way if you want to layer a snare,
      // you can do that."* Two is the layering case and is deliberately allowed;
      // a third is a producer losing a pad to a duplicate they cannot see,
      // because both copies look identical and neither says it is the spare.
      //
      // ⚠ Refused rather than swapped or auto-corrected. Moving another pad out
      // of the way would change a slot the producer did not touch, which is a
      // worse surprise than the change they asked for simply not taking.
      const already = current.filter((held, index) => index !== at && held === lane).length;
      if (already >= PAD_LIMIT) return s;

      const pads = {
        ...s.pads,
        [styleId]: current.map((held, index) => (index === at ? lane : held)),
      };
      writeStored(PADS_KEY, JSON.stringify(pads));
      return { pads };
    }),

  theme: loadThemePreference(),
  reduceMotion: loadReduceMotion(),
  language: loadLanguagePreference(),
  stemsRevealed: false,
  partsOff: [],
  looping: true,

  setActiveTab: (activeTab) => set({ activeTab }),
  toggleLooping: () => set((s) => ({ looping: !s.looping })),
  setLooping: (on) => set({ looping: on }),
  togglePart: (part) =>
    set((s) => ({
      partsOff: s.partsOff.includes(part)
        ? s.partsOff.filter((off) => off !== part)
        : [...s.partsOff, part],
    })),
  toggleRightRail: () => set((s) => ({ rightRailOpen: !s.rightRailOpen })),
  // ⛔ Only on a *crossing* — see [`lastBreakpoint`]. Called with the same
  // answer as last time this does nothing, so a manual K toggle survives.
  setWide: (wide) => {
    if (wide === lastBreakpoint) return;
    lastBreakpoint = wide;
    set({ rightRailOpen: wide });
  },

  toggleSection: (id) =>
    set((s) => {
      const sections = { ...s.sections, [id]: !s.sections[id] };
      saveSections(sections);
      return { sections };
    }),

  revealStems: () =>
    set((s) => {
      if (s.stemsRevealed) return s;
      const sections = { ...s.sections, stems: true };
      saveSections(sections);
      // ⚠ Just the fields that changed, like `toggleSection` and
      // `setAllSections` above and below. `set` merges shallowly, so spreading
      // the whole store read as though something about this setter were
      // different from its neighbours.
      return { stemsRevealed: true, sections };
    }),

  setAllSections: (open) =>
    set(() => {
      const sections = SECTIONS.reduce(
        (acc, id) => ({ ...acc, [id]: open }),
        {} as SectionState,
      );
      saveSections(sections);
      return { sections };
    }),

  setTheme: (theme) => {
    applyThemePreference(theme);
    set({ theme });
  },

  setReduceMotion: (reduceMotion) => {
    writeStored(REDUCE_MOTION_KEY, String(reduceMotion));
    set({ reduceMotion });
  },

  setLanguage: (language) => {
    applyLanguage(language);
    set({ language });
  },
}));
