import { create } from 'zustand';
import { invoke, isTauri } from '../lib/ipc';
import { writeStored } from './storage';
import { applyLanguage, loadLanguagePreference } from '../i18n';
import { isLocaleCode, type LocaleCode } from '../i18n/locales';
import {
  applyThemePreference,
  isThemePreference,
  loadThemePreference,
  type ThemePreference,
} from './theme';

/** The six generators. Order matches the tab strip in PRD § 8. */
export const GENERATOR_TABS = ['drums', 'melody', 'counter', 'bass', 'chords', 'song'] as const;

export type GeneratorTab = (typeof GENERATOR_TABS)[number];

/** Below this the right rail collapses (PRD § 8). */
export const WIDE_BREAKPOINT = 1440;

/** Individually collapsible panels. The right rail as a whole is separate — it
 *  is driven by the viewport breakpoint and the K shortcut. */
export const SECTIONS = ['genres', 'roster', 'kit', 'session'] as const;
export type SectionId = (typeof SECTIONS)[number];

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
  writeStored(SECTIONS_KEY, JSON.stringify(sections));
}

type UiState = {
  activeTab: GeneratorTab;
  /** Whether the right rail is showing. Follows the breakpoint until the user
   *  overrides it with K, which is why it is stored rather than derived. */
  rightRailOpen: boolean;
  sections: SectionState;
  theme: ThemePreference;
  language: LocaleCode;

  setActiveTab: (tab: GeneratorTab) => void;
  toggleRightRail: () => void;
  /** Called when the viewport crosses WIDE_BREAKPOINT. */
  setWide: (wide: boolean) => void;
  toggleSection: (id: SectionId) => void;
  setAllSections: (open: boolean) => void;
  setTheme: (theme: ThemePreference) => void;
  setLanguage: (language: LocaleCode) => void;
};

const startsWide = typeof window === 'undefined' ? true : window.innerWidth >= WIDE_BREAKPOINT;

export const useUi = create<UiState>((set) => ({
  activeTab: 'drums',
  rightRailOpen: startsWide,
  sections: loadSections(),
  theme: loadThemePreference(),
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

  setLanguage: (language) => {
    applyLanguage(language);
    set({ language });
  },
}));

/**
 * Reconcile the pre-paint theme with `settings.json`.
 *
 * The theme has to be applied synchronously before first paint or the window
 * flashes the wrong colours, and only localStorage can answer that fast. But
 * localStorage lives in the WebView's own profile: "clear browsing data", a
 * reset user profile, or restoring app data from a backup wipes it while
 * settings.json survives. Without this, settings.json was write-only — the
 * Settings modal saved a theme there that nothing ever read back, so the
 * durable store was decorative and the fragile one was authoritative.
 *
 * An explicit choice always beats an implicit default, whichever store holds
 * it. `system` in the file is indistinguishable from no file at all, since
 * `Settings::load` returns defaults for a missing one — so it counts as "no
 * information" rather than as a preference, and the healing runs the other way.
 */
export async function reconcileWithSettings(): Promise<void> {
  if (!isTauri()) return;
  try {
    const stored = await invoke<{ theme?: unknown; language?: unknown }>('settings_get');
    const patch: Record<string, unknown> = {};

    const themeOnDisk = isThemePreference(stored?.theme) ? stored.theme : 'system';
    const themeLocal = useUi.getState().theme;
    if (themeOnDisk !== themeLocal) {
      if (themeOnDisk !== 'system') {
        // The file has a real choice and we did not — adopt it, which also
        // rewrites localStorage so the next launch paints it immediately.
        useUi.getState().setTheme(themeOnDisk);
      } else if (themeLocal !== 'system') {
        // We have a real choice the file has never been told about: someone who
        // chose a theme before this reconcile existed, or a file that was reset.
        patch.theme = themeLocal;
      }
    }

    // Language works the same way, with the empty string as its sentinel.
    // `isLocaleCode('')` is false, so a file that has never recorded a choice
    // counts as "no information" and the local pick — which came from the OS
    // language on first launch — wins and is written back. Treating a missing
    // file as a vote for English reset every non-English machine on every
    // launch.
    const languageOnDisk = isLocaleCode(stored?.language) ? stored.language : null;
    const languageLocal = useUi.getState().language;
    if (languageOnDisk && languageOnDisk !== languageLocal) {
      useUi.getState().setLanguage(languageOnDisk);
    } else if (!languageOnDisk) {
      patch.language = languageLocal;
    }

    if (Object.keys(patch).length > 0) {
      await invoke('settings_set', { settings: { ...(stored ?? {}), ...patch } }).catch(
        () => {},
      );
    }
  } catch {
    // No bridge or no settings file yet — keep what was painted.
  }
}
