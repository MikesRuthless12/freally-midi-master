import { create } from 'zustand';
import {
  applyThemePreference,
  loadThemePreference,
  type ThemePreference,
} from './theme';

/** The six generators. Order matches the tab strip in PRD § 8. */
export const GENERATOR_TABS = [
  'drums',
  'melody',
  'counter',
  'bass',
  'chords',
  'song',
] as const;

export type GeneratorTab = (typeof GENERATOR_TABS)[number];

/** Below this the right rail collapses (PRD § 8). */
export const WIDE_BREAKPOINT = 1440;

/** Individually collapsible panels. The right rail as a whole is separate — it
 *  is driven by the viewport breakpoint and the K shortcut. */
export const SECTIONS = ['genres', 'roster', 'kit', 'session'] as const;
export type SectionId = (typeof SECTIONS)[number];

export const SECTION_LABELS: Record<SectionId, string> = {
  genres: 'Genres',
  roster: 'Roster',
  kit: 'Kit',
  session: 'Session',
};

export type SectionState = Record<SectionId, boolean>;

const SECTIONS_KEY = 'freally.sections';

const ALL_OPEN: SectionState = { genres: true, roster: true, kit: true, session: true };

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
  try {
    window.localStorage.setItem(SECTIONS_KEY, JSON.stringify(sections));
  } catch {
    // Persisting is best-effort; the in-memory choice still applies.
  }
}

type UiState = {
  activeTab: GeneratorTab;
  /** Whether the right rail is showing. Follows the breakpoint until the user
   *  overrides it with K, which is why it is stored rather than derived. */
  rightRailOpen: boolean;
  sections: SectionState;
  theme: ThemePreference;

  setActiveTab: (tab: GeneratorTab) => void;
  toggleRightRail: () => void;
  /** Called when the viewport crosses WIDE_BREAKPOINT. */
  setWide: (wide: boolean) => void;
  toggleSection: (id: SectionId) => void;
  setAllSections: (open: boolean) => void;
  setTheme: (theme: ThemePreference) => void;
};

const startsWide =
  typeof window === 'undefined' ? true : window.innerWidth >= WIDE_BREAKPOINT;

export const useUi = create<UiState>((set) => ({
  activeTab: 'drums',
  rightRailOpen: startsWide,
  sections: loadSections(),
  theme: loadThemePreference(),

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
}));
