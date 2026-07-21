/**
 * Theme preference: System (default), Dark, or Light.
 *
 * Dark is the brand's signature look and the fallback whenever the OS gives us
 * nothing to go on. The preference is written to `<html data-theme>`, which the
 * token sheet keys off; "system" removes the attribute entirely so the
 * `prefers-color-scheme` media query in tokens.css takes over.
 */

export type ThemePreference = 'system' | 'dark' | 'light';
export type ResolvedTheme = 'dark' | 'light';

const STORAGE_KEY = 'freally.theme';

export function isThemePreference(value: unknown): value is ThemePreference {
  return value === 'system' || value === 'dark' || value === 'light';
}

/** The OS-level preference, defaulting to dark when unknown or unavailable. */
export function systemTheme(): ResolvedTheme {
  if (typeof window === 'undefined' || !window.matchMedia) return 'dark';
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  return preference === 'system' ? systemTheme() : preference;
}

export function loadThemePreference(): ThemePreference {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (isThemePreference(stored)) return stored;
  } catch {
    // Private-mode or storage-disabled webviews throw on access; fall through.
  }
  return 'system';
}

/**
 * Apply a preference to the document and persist it. Explicit choices set
 * `data-theme`; "system" clears it so CSS can follow the OS on its own — that
 * way the app keeps tracking the OS if the user flips it later, with no
 * listener required.
 */
export function applyThemePreference(preference: ThemePreference): ResolvedTheme {
  const root = document.documentElement;
  if (preference === 'system') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', preference);
  }

  try {
    window.localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Persisting is best-effort; the in-memory choice still applies.
  }

  return resolveTheme(preference);
}

/** Call once at startup, before first paint, to avoid a flash of the wrong theme. */
export function initTheme(): ThemePreference {
  const preference = loadThemePreference();
  applyThemePreference(preference);
  return preference;
}
