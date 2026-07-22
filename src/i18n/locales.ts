/**
 * The canonical 18 — the Havoc-wide locale set, identical across every Freally
 * app (see locale_overhaul_plan.md).
 *
 * Two conventions carried over from the other repos, both deliberate:
 *
 * 1. **The picker shows the native endonym**, never the English name. Someone
 *    who cannot read the current UI language still has to find their own — and
 *    "Deutsch" is findable in a Japanese UI in a way that "German" is not.
 * 2. **English first, then alphabetical by ENGLISH name.** Sorting by endonym
 *    would reorder the whole list every time the language changed, so a user
 *    would never build muscle memory for where their language sits.
 */

export type LocaleCode = (typeof LOCALES)[number]['code'];

/** `english` is not shown in the UI — it exists to give the list a stable sort. */
export const LOCALES = [
  { code: 'en', english: 'English', native: 'English' },
  { code: 'ar', english: 'Arabic', native: 'العربية' },
  { code: 'zh-CN', english: 'Chinese (Simplified)', native: '简体中文' },
  { code: 'nl', english: 'Dutch', native: 'Nederlands' },
  { code: 'fr', english: 'French', native: 'Français' },
  { code: 'de', english: 'German', native: 'Deutsch' },
  { code: 'hi', english: 'Hindi', native: 'हिन्दी' },
  { code: 'id', english: 'Indonesian', native: 'Bahasa Indonesia' },
  { code: 'it', english: 'Italian', native: 'Italiano' },
  { code: 'ja', english: 'Japanese', native: '日本語' },
  { code: 'ko', english: 'Korean', native: '한국어' },
  { code: 'pl', english: 'Polish', native: 'Polski' },
  { code: 'pt-BR', english: 'Portuguese (Brazil)', native: 'Português (Brasil)' },
  { code: 'ru', english: 'Russian', native: 'Русский' },
  { code: 'es', english: 'Spanish', native: 'Español' },
  { code: 'tr', english: 'Turkish', native: 'Türkçe' },
  { code: 'uk', english: 'Ukrainian', native: 'Українська' },
  { code: 'vi', english: 'Vietnamese', native: 'Tiếng Việt' },
] as const;

export const LOCALE_CODES = LOCALES.map((l) => l.code);

/** The one right-to-left locale in the set. */
const RTL: readonly string[] = ['ar'];

export function isRtl(code: string): boolean {
  return RTL.includes(code);
}

export function isLocaleCode(value: unknown): value is LocaleCode {
  return typeof value === 'string' && (LOCALE_CODES as readonly string[]).includes(value);
}

/**
 * Pick the best supported locale for a browser/OS language tag.
 *
 * `pt-PT` should land on `pt-BR` rather than silently falling back to English:
 * a Portuguese speaker reading Brazilian Portuguese is far better served than
 * one reading English. Same for every regional variant of a language we ship.
 */
export function resolveLocale(tag: string | undefined | null): LocaleCode {
  if (!tag) return 'en';
  const wanted = tag.replace('_', '-');
  if (isLocaleCode(wanted)) return wanted;

  const base = wanted.split('-')[0].toLowerCase();
  const match = LOCALE_CODES.find((code) => code.split('-')[0].toLowerCase() === base);
  return match ?? 'en';
}
