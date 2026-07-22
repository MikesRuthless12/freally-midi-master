/**
 * i18next setup. Matches the sibling Freally apps: i18next JSON catalogs with
 * `{{var}}` placeholders, one file per locale, all bundled — nothing is fetched
 * at runtime, because the product does not touch the network.
 *
 * Every catalog is imported eagerly by `import.meta.glob`. That is a deliberate
 * choice over lazy loading: the eighteen JSON files together are far smaller
 * than one font subset, and eager loading means switching language is
 * synchronous. A language picker that shows a loading state is a worse
 * experience than one that costs a few KB in the bundle.
 */

import i18next from 'i18next';
import { initReactI18next } from 'react-i18next';

import { isLocaleCode, isRtl, LOCALE_CODES, resolveLocale, type LocaleCode } from './locales';

const STORAGE_KEY = 'freally.language';

const catalogs = import.meta.glob<{ default: Record<string, unknown> }>('./locales/*.json', {
  eager: true,
});

/** `./locales/pt-BR.json` → `pt-BR`. */
function codeFromPath(path: string): string {
  return path.replace('./locales/', '').replace('.json', '');
}

const resources = Object.fromEntries(
  Object.entries(catalogs).map(([path, module]) => [
    codeFromPath(path),
    { translation: module.default },
  ]),
);

/**
 * A missing catalog is a build error, not a runtime shrug.
 *
 * Without this a deleted or misnamed file degrades to English for that
 * language and nothing says so — the user just sees an untranslated UI and
 * assumes the app was never localised.
 */
const missing = LOCALE_CODES.filter((code) => !(code in resources));
if (missing.length > 0) {
  throw new Error(`missing locale catalogs: ${missing.join(', ')} — see src/i18n/locales/`);
}

export function loadLanguagePreference(): LocaleCode {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (isLocaleCode(stored)) return stored;
  } catch {
    // Private-mode or storage-disabled webviews throw on access.
  }
  return resolveLocale(typeof navigator === 'undefined' ? 'en' : navigator.language);
}

/**
 * Apply a language to the document and persist it.
 *
 * `lang` matters for more than screen readers: it drives the browser's own
 * font selection, hyphenation and quote marks. `dir` is what makes Arabic lay
 * out right-to-left — without it the text renders correctly and the interface
 * around it stays backwards.
 */
export function applyLanguage(code: LocaleCode): void {
  const root = document.documentElement;
  root.setAttribute('lang', code);
  root.setAttribute('dir', isRtl(code) ? 'rtl' : 'ltr');

  void i18next.changeLanguage(code);

  try {
    window.localStorage.setItem(STORAGE_KEY, code);
  } catch {
    // Persisting is best-effort; the in-memory choice still applies.
  }
}

/** Call once at startup, before first paint. */
export function initI18n(): LocaleCode {
  const language = loadLanguagePreference();

  void i18next.use(initReactI18next).init({
    resources,
    lng: language,
    fallbackLng: 'en',
    interpolation: {
      // React escapes for us; letting i18next escape as well double-encodes
      // any apostrophe or quote that appears in a translation.
      escapeValue: false,
    },
  });

  applyLanguage(language);
  return language;
}

export { i18next };
