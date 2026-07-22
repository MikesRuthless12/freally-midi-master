/**
 * The locale acceptance gate.
 *
 * Mirrors the rules in the Havoc-wide `validate_locales.py` so this repo fails
 * fast in its own CI rather than only when a cross-repo sweep runs: exactly
 * en + the canonical 17, exact key parity, identical placeholder sets per key,
 * no mojibake, and nothing left as an untranslated English copy.
 *
 * Every one of those is a real failure someone has shipped. A missing key
 * degrades silently to English, so a half-translated UI looks deliberate. A
 * dropped `{{version}}` produces "Version is available". Mojibake is what a
 * file saved as cp1252 and read as UTF-8 looks like, and it is invisible to a
 * reviewer who does not read the language. And a file copied from en.json is
 * the single most common way a locale "exists" without being translated.
 */

import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { CATEGORIES } from '../components/Settings/categories';
import { GENERATOR_TABS, SECTIONS } from '../state/ui';
import { LOCALE_CODES, LOCALES, resolveLocale } from './locales';

const dir = join(dirname(fileURLToPath(import.meta.url)), 'locales');

type Catalog = Record<string, unknown>;

function read(code: string): Catalog {
  return JSON.parse(readFileSync(join(dir, `${code}.json`), 'utf8')) as Catalog;
}

/** Flatten to `a.b.c` -> value, so parity is comparable as a flat key list. */
function flatten(value: unknown, prefix = ''): Map<string, string> {
  const out = new Map<string, string>();
  if (typeof value === 'string') {
    out.set(prefix, value);
    return out;
  }
  if (value && typeof value === 'object') {
    for (const [key, child] of Object.entries(value)) {
      for (const [k, v] of flatten(child, prefix ? `${prefix}.${key}` : key)) out.set(k, v);
    }
  }
  return out;
}

const en = flatten(read('en'));

/** `{{version}}` and friends, which must survive translation untouched. */
function placeholders(text: string): string[] {
  return [...text.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort();
}

/**
 * Terms that legitimately stay in English in every locale: brand, file formats,
 * standards bodies, third-party names. Everything else being identical to the
 * English is the signal that a string was never translated.
 */
const PRESERVED = [
  'Freally MIDI Master',
  'Freally',
  'MIDI',
  'BPM',
  'DAW',
  'WAV',
  'GitHub',
  'Gmail',
  'Lucide',
  'ISC',
  'Noto Sans',
  'SIL Open Font License 1.1',
  'CC BY 4.0',
  'Magenta Groove MIDI Dataset',
  'WCAG 2.1 AA',
  'K',
];

/** Is this string English only because every word in it is a preserved term? */
function isOnlyPreservedTerms(text: string): boolean {
  let rest = text;
  for (const term of [...PRESERVED].sort((a, b) => b.length - a.length)) {
    rest = rest.split(term).join(' ');
  }
  return rest.replace(/[\s(){}[\].,:;—–-]/g, '') === '';
}

describe('locale catalogs', () => {
  it('contains exactly the canonical 18 and nothing else', () => {
    const onDisk = readdirSync(dir)
      .filter((f) => f.endsWith('.json'))
      .map((f) => f.replace('.json', ''))
      .sort();
    expect(onDisk).toEqual([...LOCALE_CODES].sort());
  });

  it('gives every locale a distinct native name for the picker', () => {
    // Two locales showing the same label is unpickable.
    const natives = LOCALES.map((l) => l.native);
    expect(new Set(natives).size).toBe(natives.length);
  });

  it('lists English first, then alphabetically by English name', () => {
    // Sorting by endonym would reorder the list every time the UI language
    // changed, so nobody could learn where their language sits.
    const [first, ...rest] = LOCALES;
    expect(first.code).toBe('en');
    const names = rest.map((l) => l.english);
    expect(names).toEqual([...names].sort((a, b) => a.localeCompare(b, 'en')));
  });

  it('maps a regional tag onto the closest catalog we ship', () => {
    // pt-PT must reach Brazilian Portuguese rather than falling to English.
    expect(resolveLocale('pt-PT')).toBe('pt-BR');
    expect(resolveLocale('zh-TW')).toBe('zh-CN');
    expect(resolveLocale('en-GB')).toBe('en');
    expect(resolveLocale('de-AT')).toBe('de');
    expect(resolveLocale('kl-GL')).toBe('en');
    expect(resolveLocale(undefined)).toBe('en');
  });

  it('has a non-trivial English catalog to compare against', () => {
    expect(en.size).toBeGreaterThan(50);
  });

  it('defines every key the components actually ask for', () => {
    // The gap this closes: parity only compares locales *to en*, so a key the
    // code uses and no catalog defines is invisible to it — every locale agrees
    // the key is missing. i18next then renders the key itself, so the Settings
    // rail showed a tab literally reading "settings.language" and every parity
    // test stayed green.
    //
    // Only literal, non-interpolated keys can be checked statically; template
    // forms like t(`tabs.${tab}`) are covered by the prefix check below.
    const source = readdirSync(join(dir, '..', '..'), { recursive: true, encoding: 'utf8' })
      .filter((f) => /\.tsx?$/.test(f) && !/\.test\./.test(f))
      .map((f) => join(dir, '..', '..', f));

    const missing = new Set<string>();

    for (const file of source) {
      let text: string;
      try {
        text = readFileSync(file, 'utf8');
      } catch {
        continue; // a directory entry, not a file
      }
      for (const [, key] of text.matchAll(/\bt\(\s*'([a-zA-Z0-9_.]+)'/g)) {
        if (!en.has(key)) missing.add(key);
      }
    }

    expect([...missing].sort()).toEqual([]);
  });

  /**
   * The keys built by interpolation, which the scan above cannot see.
   *
   * `t(`settings.${id}`)` is invisible to a static search for `t('...')`, so a
   * category with no catalog entry renders its own key — the Settings rail
   * showed a tab reading "settings.language" while every other test stayed
   * green. Asserting against the same constants the components iterate is the
   * only way to catch that: add a tab, forget the string, and this fails.
   */
  it.each([
    ['settings', CATEGORIES],
    ['tabs', GENERATOR_TABS],
    ['sections', SECTIONS],
    ['theme', ['system', 'dark', 'light']],
  ] as const)('defines a %s entry for every value the UI iterates', (group, values) => {
    const missing = values.filter((value) => !en.has(`${group}.${value}`));
    expect(missing).toEqual([]);
  });

  describe.each(LOCALE_CODES.filter((c) => c !== 'en'))('%s', (code) => {
    const catalog = flatten(read(code));

    it('has exactly the same keys as en', () => {
      const missing = [...en.keys()].filter((k) => !catalog.has(k));
      const extra = [...catalog.keys()].filter((k) => !en.has(k));
      expect({ missing, extra }).toEqual({ missing: [], extra: [] });
    });

    it('keeps every placeholder from en', () => {
      const wrong: Record<string, { en: string[]; got: string[] }> = {};
      for (const [key, english] of en) {
        const translated = catalog.get(key) ?? '';
        const expected = placeholders(english);
        const actual = placeholders(translated);
        if (expected.join() !== actual.join()) wrong[key] = { en: expected, got: actual };
      }
      expect(wrong).toEqual({});
    });

    it('has no empty strings', () => {
      const blank = [...catalog].filter(([, v]) => v.trim() === '').map(([k]) => k);
      expect(blank).toEqual([]);
    });

    it('is not mojibake', () => {
      // The signature of UTF-8 bytes decoded as latin-1/cp1252, plus stray
      // replacement characters. Either means the file was written or converted
      // with the wrong encoding, and it is unreadable to a native speaker.
      const damaged = [...catalog].filter(([, v]) => /[ÂÃ][-¿]|�/.test(v)).map(([k]) => k);
      expect(damaged).toEqual([]);
    });

    it('is actually translated, not a copy of en', () => {
      const untouched = [...en]
        .filter(([key, english]) => catalog.get(key) === english)
        .filter(([, english]) => !isOnlyPreservedTerms(english))
        .map(([key]) => key);

      // A handful of one-word labels can legitimately coincide (a language may
      // genuinely use "Loop"), but a wholesale match means the file was copied.
      expect(untouched.length).toBeLessThan(Math.ceil(en.size * 0.1));
    });
  });
});
