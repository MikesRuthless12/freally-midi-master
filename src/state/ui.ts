import type { CSSProperties } from 'react';
import { create } from 'zustand';
import { readStored, writeStored } from './storage';
import { applyLanguage, loadLanguagePreference } from '../i18n';
import { type LocaleCode } from '../i18n/locales';
import { applyThemePreference, loadThemePreference, type ThemePreference } from './theme';
import { DECADES, type Decade } from '../lib/era';
import type { Lane } from '../lib/ipc-types';
// ⚠ `state/lanes.ts` is a leaf that imports nothing but a type, which is why it
// can be reached from here without dragging the kit store — and its own doc
// records what happened the last time that list lived somewhere less reachable.
import { PAD_LANES } from './lanes';

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

// ⚠ **`freally.sections` is gone with the accordions it described.** The rails
// store which *group* each is showing now — see `GROUPS_KEY` below, which is a
// new key on purpose: the old value is a boolean per panel, and every existing
// install has all eight set true, which is the one layout this design cannot
// draw.

const REDUCE_MOTION_KEY = 'freally.reduceMotion';
/**
 * Whether the producer has already been shown where their stems went.
 *
 * ⛔⛔ **NOT PERSISTING THIS OVERWROTE A SAVED RAIL CHOICE ON EVERY LAUNCH.**
 * `revealStems` switches the right rail to `kit · stems` the first time anything
 * is generated — a one-off nudge, and a good one. But the flag lived only in
 * memory while the rail choice is stored under `freally.railGroups`, so every
 * reload made it "the first time" again: open a project with a saved pattern in
 * it and the subscriber fired before the producer touched anything, taking away
 * the `session · presets · pattern library` group they had chosen and *saving*
 * the replacement. The preference could never come back.
 */
const STEMS_REVEALED_KEY = 'freally.stemsRevealed';
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
 * The default layout as one array that is **the same object every time**.
 *
 * ⛔⛔ **THE FRESH-ARRAY FALLBACK IS AN INFINITE RENDER**, and it cost a blank
 * window Mike screenshotted on 2026-08-09. `padsFor` used to answer
 * `[...DEFAULT_PADS]`, so `useUi(s => s.padsFor(id))` returned a **new array on
 * every call**, zustand's equality check never held, and the component
 * re-rendered until React gave up.
 *
 * ▶ **Three components then each carried their own module constant plus their
 * own copy of the warning** — `PadGrid`, `KitPanel` and `ExplorerPanel` — because
 * the store was handing out the loaded gun and every consumer had to know not to
 * pull the trigger. Fixing it here is what let all three delete theirs.
 *
 * ⚠ **Frozen**, so "the same object every time" cannot quietly stop being true:
 * anything that mutates the fallback in place would be corrupting every style
 * that has not been customised, and it now throws instead.
 */
const FALLBACK_PADS: readonly string[] = Object.freeze([...DEFAULT_PADS]);

/**
 * The key a pad layout is stored under before an artist has been chosen.
 *
 * ⛔⛔ **`null` USED TO MEAN "REFUSE THE EDIT", AND THAT WAS MIKE'S BUG.**
 * `setPad` and `movePad` both began `if (styleId === null) return s`, so on a
 * fresh install — or any moment before a style is picked, which `PadGrid`'s own
 * comment records as reachable because the roster combobox *shows* an artist
 * while `selectedId` is still null — the grid drew eight editable pads and every
 * drag silently snapped back. That is his report: *"the drum pads do not let me
 * reorder them."*
 *
 * ▶ **So there is no null case any more, only a style with an empty name.** The
 * readers already keyed on `selectedId ?? ''` before `padsOf` existed; the
 * writers did not, and the two were consistent only because nothing ever wrote
 * to `''`.
 *
 * ⚠ Picking an artist afterwards starts from the default again, which is the
 * per-style rule Mike asked for — *"the original workflows should go back to
 * exactly how they were when you left them"* — rather than a fault in this.
 */
const NO_STYLE = '';

/**
 * The eight lanes a style's pads address, defaulted when it has none.
 *
 * ⚠ **Safe inside a zustand selector**, which is the whole point — see
 * [`FALLBACK_PADS`]. Pass `s.pads`, never `s`, so the selector re-runs only when
 * the map itself changes.
 */
export function padsOf(
  pads: Record<string, string[]>,
  styleId: string | null,
): readonly string[] {
  return pads[styleId ?? NO_STYLE] ?? FALLBACK_PADS;
}

/**
 * ⛔ **Validated against the real lane list on the way in, not trusted.** This is
 * localStorage: it survives updates, and a lane renamed or removed by a later
 * release would otherwise leave a pad addressing something that no longer
 * exists — a pad that silently does nothing, which is the worst kind. Anything
 * unrecognised falls back to the default for that slot.
 *
 * ⛔⛔ **`PAD_LANES`, not `ALL_LANES`, and the difference is a blank picker.** The
 * pad combobox was narrowed to the non-melodic lanes when the melodic ones got
 * their own generators; this kept accepting all of them, so a layout saved under
 * the older build could hold `melody` — a value `Combo` finds no option for, so
 * it renders its placeholder over a tile that is still wired to and audibly
 * playing that lane. There is no migration for `freally.pads`, so this *is* the
 * migration: an unusable value is read back as the slot's default.
 */
function cleanPads(parsed: unknown): string[] {
  const fallback = [...DEFAULT_PADS];
  if (!Array.isArray(parsed)) return fallback;
  return fallback.map((lane, at) =>
    typeof parsed[at] === 'string' && PAD_LANES.includes(parsed[at] as Lane)
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

/**
 * The panels each rail can show, **as groups that swap together**.
 *
 * ⛔⛔ **THIS IS MIKE'S MODEL, GIVEN 2026-08-11, AND IT IS NOT "TWO PANELS AT A
 * TIME".** The first cut of this was: any two panels per rail, with a third
 * click evicting the least-recently-touched one. He replaced it with something
 * simpler and better: *"have the roster and genres showing, and file explorer's
 * vertical tab replaces and takes the place of both roster and genres, and on
 * the right kits and stems show first, and session, presets and patterns show
 * when you click and switch places with kits and stems … because kits and stems
 * are your biggest ones that take up the most space, the other 3 can fit as a
 * unit."*
 *
 * ▶ **Why the group is the right unit and the slot was not.** The panels are not
 * interchangeable tiles. KIT is 21 lanes and STEMS is five drag rows with a
 * menu — each wants half a rail. SESSION, PRESETS and PATTERN LIBRARY are chips,
 * a combobox and a short list, and all three fit in the space those two need. A
 * per-panel slot rule cannot express that; it would happily pair KIT with
 * SESSION and leave two thirds of the rail empty, or evict STEMS for a preset
 * picker. Grouping says *these belong together at these sizes*, which is the
 * thing that was actually true.
 *
 * ⚠ **A group is also why there is no eviction rule left.** Nothing has to be
 * closed to make room, so there is no "which one goes" to get wrong — a rail
 * shows exactly one group, always, and a tab picks it.
 *
 * ⚠ **Every panel still has its own tab.** Clicking any of them brings in the
 * group it belongs to; the whole group then reads as on. Naming the strip after
 * groups would hide five of the eight panel names behind two labels.
 */
export const RAIL_GROUPS = {
  left: [['genres', 'roster'], ['explorer']],
  right: [
    ['kit', 'stems'],
    ['session', 'presets', 'patterns'],
  ],
} as const satisfies Record<string, readonly (readonly SectionId[])[]>;

export type RailId = keyof typeof RAIL_GROUPS;

export const RAIL_IDS = ['left', 'right'] as const satisfies readonly RailId[];

/** The rail a panel lives in, and which group of it. */
export function groupOf(id: SectionId): { rail: RailId; at: number } {
  for (const rail of RAIL_IDS) {
    const at = (RAIL_GROUPS[rail] as readonly (readonly SectionId[])[]).findIndex((group) =>
      group.includes(id),
    );
    if (at !== -1) return { rail, at };
  }
  // Unreachable while `SECTIONS` and `RAIL_GROUPS` agree, which the tests pin.
  return { rail: 'right', at: 0 };
}

/** Which group each rail is showing. */
export type OpenGroups = Record<RailId, number>;

/**
 * How long a rail takes to swap one group for another, in milliseconds.
 *
 * ⛔ **Two stages, and the second waits for the first** — Mike, 2026-08-11:
 * *"slide one panel to the right and out and then slide the other one out"* …
 * *"you can actually see it hiding and can visibly see the other one starting to
 * slide out."* So this is `out + in`, and `layout.css` splits it: the leaving
 * group animates for [`SWAP_OUT_MS`] and the arriving one is delayed by exactly
 * that before it starts. Long enough to read as a movement, short enough that
 * clicking a tab still feels like pressing a button.
 *
 * ⚠ **The three numbers live here and are consumed as custom properties**, so
 * the clock that unmounts the old group cannot drift from the animation that
 * hides it — a shorter timer is the old panels blinking out mid-slide, a longer
 * one is them sitting there after they finished.
 */
export const SWAP_OUT_MS = 190;
export const SWAP_IN_MS = 230;
export const SWAP_MS = SWAP_OUT_MS + SWAP_IN_MS;

/** The pending unmount per rail. Module-level: one clock per rail, always. */
const swapTimers: Record<RailId, number | undefined> = { left: undefined, right: undefined };

/**
 * The swap durations, as custom properties for `layout.css` to animate on.
 *
 * ⛔ **Handed to the stylesheet rather than restated in it.** The animation that
 * hides the old group and the `setTimeout` that unmounts it are the same length
 * by definition; written in two files they would be the same length only by
 * agreement, and the failure is invisible in tests — a few milliseconds short is
 * the old panels blinking out mid-slide, a few long is them sitting there after
 * the new ones have arrived. Both read as "the animation is janky" rather than
 * as a number being wrong.
 */
export const SWAP_STYLE = {
  '--rail-swap-out': `${SWAP_OUT_MS}ms`,
  '--rail-swap-in': `${SWAP_IN_MS}ms`,
} as CSSProperties;

/**
 * What the rails show on a fresh install.
 *
 * ⛔ **Mike named all of it:** *"roster and genres should be shown and file
 * explorer should be hidden for the left hand side and for the right, kits and
 * stems should be shown and the rest should be hidden to start the app."* Which
 * is group 0 of each rail — the ordering above is the answer, so there is no
 * second list here to disagree with it.
 */
const DEFAULT_GROUPS: OpenGroups = { left: 0, right: 0 };

/** The panels a set of open groups puts on screen. */
export function sectionsFor(open: OpenGroups): SectionState {
  const showing = new Set<SectionId>(
    RAIL_IDS.flatMap((rail) => {
      const groups: readonly (readonly SectionId[])[] = RAIL_GROUPS[rail];
      return [...(groups[open[rail]] ?? groups[0] ?? [])];
    }),
  );
  return SECTIONS.reduce((acc, id) => ({ ...acc, [id]: showing.has(id) }), {} as SectionState);
}

/**
 * ⚠ **A new key, and the old one is deliberately not migrated.** `freally.sections`
 * held a boolean per panel from the accordion model — a shape with no meaning
 * here, and every existing install has all eight set true, which reads as "show
 * everything" and is the one thing this layout cannot do. Reading it would mean
 * guessing which group those eight booleans meant. Starting from the default
 * pair is both honest and what Mike asked for.
 */
const GROUPS_KEY = 'freally.railGroups';

function loadGroups(): OpenGroups {
  try {
    const raw = window.localStorage.getItem(GROUPS_KEY);
    if (!raw) return { ...DEFAULT_GROUPS };
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) return { ...DEFAULT_GROUPS };
    const stored = parsed as Partial<Record<RailId, unknown>>;
    const out = { ...DEFAULT_GROUPS };
    for (const rail of RAIL_IDS) {
      const at = stored[rail];
      // ⚠ Range-checked, not merely type-checked: a stored index from a build
      // with more groups would leave the rail showing nothing at all.
      if (
        typeof at === 'number' &&
        Number.isInteger(at) &&
        at >= 0 &&
        at < RAIL_GROUPS[rail].length
      ) {
        out[rail] = at;
      }
    }
    return out;
  } catch {
    return { ...DEFAULT_GROUPS };
  }
}

function saveGroups(open: OpenGroups): void {
  writeStored(GROUPS_KEY, JSON.stringify(open));
}

type UiState = {
  activeTab: GeneratorTab;
  /** Whether the right rail is showing. Follows the breakpoint until the user
   *  overrides it with K, which is why it is stored rather than derived. */
  rightRailOpen: boolean;
  /**
   * Whether the generator stage — the tabs, the grid, the roll — is showing.
   *
   * ⛔⛔ **Mike, 2026-08-12**: *"can we make the center window collapsible, so
   * that way if you have a generation that you like, that the center can be
   * collapsed so that the 'right rail' will always show … like the whole
   * generation part."* Once a take is settled the stage is the part you have
   * finished with, and STEMS is the part you still need — this lets the two
   * rails own the window so a small one is still a working one.
   *
   * ⚠ **Not persisted, unlike the panel groups.** Collapsing the thing you
   * generate in is a moment ("I like this, now let me drag it out"), not a
   * layout preference, and an app that reopened with its stage hidden would look
   * broken to the producer who did it once yesterday.
   */
  stageOpen: boolean;
  /**
   * Which panels are on screen.
   *
   * ⛔ **Derived from [`openGroups`] and never written on its own.** It is what
   * every consumer already read before the rails became groups, so keeping it
   * meant `Section`, the View menu and the tests did not have to learn a second
   * concept — and computing it on every write is what stops the two from ever
   * describing different layouts.
   */
  sections: SectionState;
  /** Which group each rail is showing. See `RAIL_GROUPS`. */
  openGroups: OpenGroups;
  /**
   * The group each rail is animating *away*, or null.
   *
   * ⛔ **What keeps the outgoing panels mounted long enough to be seen leaving.**
   * Without it React drops them the frame the swap starts and only the arriving
   * group can move — see `showSection`.
   */
  leaving: Record<RailId, number | null>;
  /**
   * Every style's pad layout, by style id.
   *
   * ⚠ **Select this and pick with [`padsOf`]** — never a selector that indexes
   * it, which is the infinite-render trap [`padsOf`]'s own doc records. There
   * was a `padsFor` accessor on this store for the same job; every reader now
   * takes the map and calls `padsOf`, so it had no callers left.
   */
  pads: Record<string, string[]>;
  /** Put a different lane on one of this style's pads. Persisted immediately. */
  setPad: (styleId: string | null, at: number, lane: string) => void;
  /**
   * Which pad the keyboard is aimed at, as a slot index.
   *
   * ⛔⛔ **Never null, and Mike said why** (2026-08-11): *"there has to be a way
   * to select the drum pad so you know which one you are putting it into"* and
   * *"one has to always be selected no matter what, so that way you aren't
   * trying to force one into something that's not there."* A number rather than
   * an optional lane, so the target always exists and `Ctrl`+arrow can never be
   * a gesture aimed at nothing.
   *
   * ⚠ **The slot, not the lane.** Two pads may hold the same lane — the layering
   * case `PAD_LIMIT` allows — so a lane cannot name a pad. It is also what makes
   * the selection survive a pad being pointed at a different lane.
   */
  selectedPad: number;
  selectPad: (at: number) => void;
  /**
   * Move a pad to a different slot, carrying its lane with it.
   *
   * ⛔ **Mike, 2026-08-11:** *"you should be able to click and drag your drum
   * pads and replace the ordering, and the 'Kits' rail on the right should
   * reorder with them"* — *"and the ordering should persist from unload to
   * reload of the app/vst."* All three fall out of writing through `pads`: the
   * grid renders from it, `KitPanel` sorts by it, and `setPad` already persists
   * it to the same key.
   */
  movePad: (styleId: string | null, from: number, to: number) => void;
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
   * The era pills a producer has pressed, if any (TASK-158G).
   *
   * ⛔⛔ **Mike, 2026-08-10:** *"allow the end user to [filter] the list by what
   * genre/artist was out within those specific years instead of trying to search
   * through them all and not finding what you want and just randomly searching
   * for names through genres/artists/producers blindly."*
   *
   * ⚠ **Empty means no filter, never "nothing matches"** — see `lib/era.ts`.
   *
   * ⛔ **UI state, and unlike `partsOff` it stays here.** That one moved into the
   * session document because an *import* writes it, so it became a statement
   * about the record. This is a way of looking at a list: it changes nothing
   * about what is generated, nothing about what a project contains, and reopening
   * a project with somebody's browsing filter still applied would be a control
   * left on with no memory of pressing it.
   */
  eras: Decade[];
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
  /** Press an era pill, or press it again to release it (TASK-158G). */
  toggleEra: (decade: Decade) => void;
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
  /** Collapse the generator stage so the rails own the window, or bring it back. */
  toggleStage: () => void;
  /** Called when the viewport crosses WIDE_BREAKPOINT. */
  setWide: (wide: boolean) => void;
  /**
   * Bring a panel on screen by showing the group it belongs to.
   *
   * ⚠ **Not a toggle**, which is why it is not called one any more: a rail
   * always shows exactly one group, so there is no state in which a panel can be
   * dismissed on its own. Clicking one already showing does nothing.
   */
  showSection: (id: SectionId) => void;
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

/**
 * The CSS `zoom` the plugin has put on the root, or 1 where there is none.
 *
 * ⛔⛔ **FOR PLACING A `position: fixed` PORTAL, AND NOTHING ELSE.** The rule it
 * exists for is a units mismatch that only bites inside the plugin:
 * `getBoundingClientRect()` answers in **viewport** pixels — the zoom is already
 * in the number — while a `top`/`left` written onto a fixed element is resolved
 * in that element's **own** coordinates and then multiplied by the inherited
 * zoom. So measuring a trigger and copying the figure straight across places the
 * menu at `rect × zoom`, and the further right the trigger is the further off it
 * lands. A chip in the *right rail* — the Stems panel — is the worst case in the
 * window, which is why the drum-lane menu was the first thing to go missing while
 * a combobox at the left edge still looked fine.
 *
 * ▶ Mike, 2026-08-11, on the VST3 in Ableton: *"the 'Drums Pattern' MIDI doesn't
 * let me drag anything in."* The menu opens; it is simply not where the pointer
 * is. ⚠ **That is the symptom, not the cause** — the root cause was the webview
 * being bounded to Ableton's own window, which is what made the zoom anything
 * other than 1 (`nih-plug-webview/src/lib.rs`, the note in `on_frame`). This
 * guard is what stops a wrong zoom taking the menus with it next time.
 *
 * ⚠ **A no-op at `zoom: 1`, which is every browser and the standalone.** The
 * root carries no inline zoom until `WindowFit` sets one, so `style.zoom` is the
 * empty string and this answers 1 — dividing by it changes nothing at all.
 */
export function rootZoom(): number {
  if (typeof document === 'undefined') return 1;
  const zoom = Number(document.documentElement.style.zoom);
  // ⚠ Bounded rather than trusted. Mid-resize a host can leave a figure that
  // makes the division absurd, and a menu placed at `Infinity` is a menu the
  // producer cannot find at all — the failure this function exists to prevent.
  return Number.isFinite(zoom) && zoom >= 0.2 && zoom <= 4 ? zoom : 1;
}

const startsWide = typeof window === 'undefined' ? true : isWide();

/**
 * The last answer the breakpoint gave, so repeating it cannot undo a K toggle.
 *
 * ⛔⛔ **The crossing check belongs here, because more than one caller has it to
 * get right and one of them did not.** `setWide` used to be an unconditional
 * `set({ rightRailOpen: wide })`, and `App.tsx`'s resize listener guarded it by
 * hand — a guard `WindowFit.tsx::applyZoom` did not have. Once `applyZoom`
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

/** Read once at start-up and used for both `openGroups` and `sections`. */
const initialGroups = loadGroups();

export const useUi = create<UiState>((set) => ({
  activeTab: 'drums',
  rightRailOpen: startsWide,
  stageOpen: true,
  // ⛔ **Derived, never set directly.** `openGroups` is the truth; this is what
  // every consumer already reads, so keeping it means `Section`, the View menu
  // and the tests did not have to learn about groups. The pair can only ever
  // agree because one is computed from the other on every write.
  // ⚠ One read of localStorage, not two: `sectionsFor` used to be handed a
  // second `loadGroups()` call, which parsed the same JSON again to reach the
  // same answer.
  openGroups: initialGroups,
  leaving: { left: null, right: null },
  sections: sectionsFor(initialGroups),
  pads: loadPads(),

  setPad: (styleId, at, lane) =>
    set((s) => {
      // ⚠ `NO_STYLE` rather than a refusal — see its own note for the report
      // that came of returning early here.
      const key = styleId ?? NO_STYLE;
      const current = s.pads[key] ?? [...FALLBACK_PADS];

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
        [key]: current.map((held, index) => (index === at ? lane : held)),
      };
      writeStored(PADS_KEY, JSON.stringify(pads));
      return { pads };
    }),

  selectedPad: 0,

  // ⚠ Clamped rather than trusted. The only callers are the grid's own handlers,
  // but a selection pointing past the end is a target that does not exist, which
  // is the one thing this value promises never to be.
  selectPad: (at) =>
    set({ selectedPad: Math.max(0, Math.min(DEFAULT_PADS.length - 1, Math.trunc(at))) }),

  movePad: (styleId, from, to) =>
    set((s) => {
      // ⚠ `NO_STYLE` rather than a refusal — see its own note. This is the exact
      // line Mike's "the drum pads do not let me reorder them" came out of on a
      // session with no artist chosen.
      const key = styleId ?? NO_STYLE;
      const current = s.pads[key] ?? [...FALLBACK_PADS];
      if (from === to || from < 0 || to < 0 || from >= current.length || to >= current.length) {
        return s;
      }

      // ⛔ **Lifted out and re-inserted, not swapped.** Swapping two pads moves a
      // lane the producer never touched into the slot they dragged *from*, which
      // reads as the grid shuffling itself. Dropping pad 1 onto pad 5 should
      // leave 2, 3 and 4 in the same order one place earlier — the behaviour
      // every reorderable list has.
      const next = [...current];
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);

      const pads = { ...s.pads, [key]: next };
      // Same key and same moment as `setPad`, so a reordering survives a reload
      // exactly as a lane change does.
      writeStored(PADS_KEY, JSON.stringify(pads));

      // ⚠ **The selection travels with the pad it is on.** Leaving the index put
      // would mean dragging the selected pad silently re-aims the keyboard at
      // whatever slid into its slot.
      const selectedPad =
        s.selectedPad === from
          ? to
          : s.selectedPad > from && s.selectedPad <= to
            ? s.selectedPad - 1
            : s.selectedPad < from && s.selectedPad >= to
              ? s.selectedPad + 1
              : s.selectedPad;

      return { pads, selectedPad };
    }),

  theme: loadThemePreference(),
  reduceMotion: loadReduceMotion(),
  language: loadLanguagePreference(),
  stemsRevealed:
    readStored(STEMS_REVEALED_KEY, (v): v is string => v === 'true', 'false') === 'true',
  eras: [],
  looping: true,

  setActiveTab: (activeTab) => set({ activeTab }),
  // ⚠ Kept in `DECADES` order rather than press order, so the pills read the
  // same whichever way round they were pressed.
  toggleEra: (decade) =>
    set((s) => ({
      eras: s.eras.includes(decade)
        ? s.eras.filter((held) => held !== decade)
        : DECADES.filter((held) => held === decade || s.eras.includes(held)),
    })),
  toggleLooping: () => set((s) => ({ looping: !s.looping })),
  setLooping: (on) => set({ looping: on }),
  toggleRightRail: () => set((s) => ({ rightRailOpen: !s.rightRailOpen })),
  // ⛔⛔ **Collapsing the stage OPENS the right rail, and without that the
  // feature does nothing at the size it was asked for.** Mike, 2026-08-12:
  // *"the center can be collapsed so that the 'right rail' will always show
  // because right now if you resize to too small of a size, the right rail is
  // never visible."* The second half is a separate mechanism from the first:
  // `WIDE_BREAKPOINT` auto-closes the rail below 1440, so at the small window he
  // was describing the rail is **not mounted at all** — collapsing the stage
  // there would have left him looking at the left rail and a hole where he
  // expected STEMS.
  //
  // ⚠ **Only on the way in.** Bringing the stage back does not close the rail:
  // that would undo a choice the producer may have made by hand, and there is
  // room for both once the window is wide enough to want the stage again.
  toggleStage: () =>
    set((s) => (s.stageOpen ? { stageOpen: false, rightRailOpen: true } : { stageOpen: true })),
  // ⛔ Only on a *crossing* — see [`lastBreakpoint`]. Called with the same
  // answer as last time this does nothing, so a manual K toggle survives.
  setWide: (wide) => {
    if (wide === lastBreakpoint) return;
    lastBreakpoint = wide;
    set({ rightRailOpen: wide });
  },

  // ⛔⛔ **A tab SWAPS ITS GROUP IN; nothing is closed** — Mike, 2026-08-11:
  // *"file explorer's vertical tab replaces and takes the place of both roster
  // and genres … session, presets and patterns show when you click and switch
  // places with kits and stems."*
  //
  // ⚠ **The name stayed `toggleSection` even though it no longer toggles**, and
  // that is a deliberate trade: `Section`, `ViewMenu` and the shortcut all call
  // it, and renaming would touch four files to say the same thing. What it does
  // is on `RAIL_GROUPS`.
  //
  // ⚠ **Clicking a panel already on screen does nothing.** There is no state
  // where a rail shows no group — every group is somebody's home, and a rail
  // that could empty itself would leave its tabs pointing at a blank column.
  showSection: (id) =>
    set((s) => {
      const { rail, at } = groupOf(id);
      if (s.openGroups[rail] === at) return s;
      const openGroups = { ...s.openGroups, [rail]: at };
      saveGroups(openGroups);

      // ⛔⛔ **THE GROUP ON ITS WAY OUT STAYS MOUNTED, AND THAT IS THE WHOLE
      // ANIMATION** — Mike, 2026-08-11: *"you need to have a cool switch out,
      // like slide one panel to the right and out and then slide the other one
      // out"* … *"do it so it looks cool as it's doing it, like you can actually
      // see it hiding and can visibly see the other one starting to slide out."*
      //
      // React unmounts a panel the instant it stops being in the open group, so
      // there is nothing left on screen to animate *away* — the old set would
      // vanish and only the new one could move. Recording which group is leaving
      // is what keeps it rendered for the length of its exit; `Section` reads
      // this to decide whether it is coming, going, or gone.
      //
      // ⚠ **Cleared by a timer rather than by `animationend`.** The exiting
      // panels are several elements and the event fires per element; the
      // shortest of them would take the whole group off screen mid-flight. One
      // clock, owned by the thing that started the swap.
      const leaving = { ...s.leaving, [rail]: s.openGroups[rail] };
      window.clearTimeout(swapTimers[rail]);
      swapTimers[rail] = window.setTimeout(() => {
        useUi.setState((live) => ({ leaving: { ...live.leaving, [rail]: null } }));
      }, SWAP_MS);

      return { openGroups, leaving, sections: sectionsFor(openGroups) };
    }),

  revealStems: () =>
    set((s) => {
      if (s.stemsRevealed) return s;
      // ⚠ Recorded before anything else, and on disk — see `STEMS_REVEALED_KEY`.
      // Once is the whole contract: "here is where your stems went", not a rail
      // policy that reasserts itself.
      writeStored(STEMS_REVEALED_KEY, 'true');

      // ⚠ Through the same door as a tab click, so revealing Stems brings its
      // whole group rather than putting one panel somewhere nothing else is.
      const { rail, at } = groupOf('stems');
      const openGroups = { ...s.openGroups, [rail]: at };

      // ⛔ **NOT `saveGroups`.** A tab press is the producer choosing; this is the
      // app interrupting to show them something. Writing it through would make
      // the interruption outlive the session and replace a choice they had
      // already made — see the key's own note. If they leave the rail here and
      // then press a tab, *that* is what gets stored.

      // ⚠ Just the fields that changed. `set` merges shallowly, so spreading the
      // whole store read as though something about this setter were different
      // from its neighbours.
      return { stemsRevealed: true, openGroups, sections: sectionsFor(openGroups) };
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
