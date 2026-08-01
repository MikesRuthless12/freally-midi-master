import { create } from 'zustand';
import { readStored, writeStored } from './storage';
import { applyLanguage, loadLanguagePreference } from '../i18n';
import { type LocaleCode } from '../i18n/locales';
import { applyThemePreference, loadThemePreference, type ThemePreference } from './theme';

/** The six generators. Order matches the tab strip in PRD § 8. */
export const GENERATOR_TABS = ['drums', 'melody', 'counter', 'bass', 'chords', 'song'] as const;

export type GeneratorTab = (typeof GENERATOR_TABS)[number];

/** Below this the right rail collapses (PRD § 8). */
export const WIDE_BREAKPOINT = 1440;

/** Individually collapsible panels. The right rail as a whole is separate — it
 *  is driven by the viewport breakpoint and the K shortcut. */
export const SECTIONS = ['genres', 'roster', 'kit', 'session', 'presets'] as const;
export type SectionId = (typeof SECTIONS)[number];

export type SectionState = Record<SectionId, boolean>;

const SECTIONS_KEY = 'freally.sections';

const REDUCE_MOTION_KEY = 'freally.reduceMotion';

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
  kit: true,
  session: true,
  presets: true,
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

  setActiveTab: (tab: GeneratorTab) => void;
  toggleRightRail: () => void;
  /** Called when the viewport crosses WIDE_BREAKPOINT. */
  setWide: (wide: boolean) => void;
  toggleSection: (id: SectionId) => void;
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
export function isWide(): boolean {
  if (typeof document === 'undefined') return true;
  return (document.documentElement.clientWidth || window.innerWidth) >= WIDE_BREAKPOINT;
}

const startsWide = typeof window === 'undefined' ? true : isWide();

export const useUi = create<UiState>((set) => ({
  activeTab: 'drums',
  rightRailOpen: startsWide,
  sections: loadSections(),
  theme: loadThemePreference(),
  reduceMotion: loadReduceMotion(),
  language: loadLanguagePreference(),

  setActiveTab: (activeTab) => set({ activeTab }),
  toggleRightRail: () => set((s) => ({ rightRailOpen: !s.rightRailOpen })),
  setWide: (wide) => set({ rightRailOpen: wide }),

  toggleSection: (id) =>
    set((s) => {
      const sections = { ...s.sections, [id]: !s.sections[id] };
      saveSections(sections);
      return { sections };
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
