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

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { CATEGORIES } from '../components/Settings/categories';
import { ALL_LANES } from '../state/kit';
import { SHORTCUT_GROUPS } from '../components/Shortcuts/catalog';
import { SCALES } from '../components/SessionChips/values';
import { THEME_PREFERENCES } from '../state/theme';
import { SECTION_KINDS } from '../components/SongTimeline/clips';
import { GENERATOR_TABS, SECTIONS } from '../state/ui';
import { LOCALE_CODES, LOCALES, resolveLocale } from './locales';
import type { SplitReason } from '../lib/ipc-types';

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

/** Every non-test .ts/.tsx file under src/, for scanning t() calls. */
function sourceFiles(): string[] {
  const root = join(dir, '..', '..');
  return readdirSync(root, { recursive: true, encoding: 'utf8' })
    .filter((f) => /\.tsx?$/.test(f) && !/\.test\./.test(f))
    .map((f) => join(root, f))
    .filter((f) => statSync(f).isFile());
}

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
  // ⛔ **Scale names that are proper nouns** (TASK-041C). These are the names
  // of specific Japanese, Indian and Balinese scales, and they are written the
  // same way in every locale — Ableton's own scale menu does exactly this.
  // Inventing a translation would produce a name no musician would recognise
  // and no search would find, which is the failure this list exists to allow
  // an exception for. The *descriptive* scales beside them are all translated.
  'Bhairav',
  'Hirajoshi',
  'In-Sen',
  'Iwato',
  'Kumoi',
  'Pelog Selisir',
  'Pelog Tembung',
];

/** Is this string English only because every word in it is a preserved term? */
function isOnlyPreservedTerms(text: string): boolean {
  let rest = text;
  for (const term of [...PRESERVED].sort((a, b) => b.length - a.length)) {
    rest = rest.split(term).join(' ');
  }
  return rest.replace(/[\s(){}[\].,:;—–-]/g, '') === '';
}

/** Every `Tier` the bindings define. A union has no runtime form to iterate. */
const TIERS = ['flagship', 'standard', 'inherited'] as const;

/**
 * Every reason a layered `.mid` can be routed by (TASK-058D).
 *
 * ⚠ **Restated here rather than imported**, because the source of truth is a
 * Rust enum — `engine::smf_read::SplitReason` — and `ipc-types.ts` exports it as
 * a union type, which has no runtime value to iterate. Adding a variant in Rust
 * and forgetting this list fails the templated-prefix test below rather than
 * rendering `explorer.splitReason.whatever` at the producer.
 */
const SPLIT_REASONS = [
  'drumChannel',
  'kitShape',
  'polyphonic',
  'lowestVoice',
  'highestVoice',
  'innerVoice',
  'splitByPitch',
  'fromName',
] as const satisfies readonly SplitReason[];

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

  it('⛔ says none of the five words the voice guide forbids (TASK-089)', () => {
    // ⛔⛔ **A hard rule, quoted from `docs/product-vision.md` § Voice & Tone:**
    // *"never use the words 'leverage,' 'unlock,' 'empower,' 'solution,' or
    // 'AI-powered' anywhere a user can see."* The second half of that sentence
    // is why this is a test rather than a review note — "anywhere a user can
    // see" is 18 catalogs and ~900 strings, which nobody re-reads.
    //
    // ⚠ **English only, and that is the honest scope.** The rule is about the
    // register of English startup copy; the equivalent in Japanese is not a word
    // list this repo can hold, and a transliteration gate would fail on ordinary
    // Polish. What it does catch is the way these words actually arrive: someone
    // writes one in `en.json` and seventeen translators render it faithfully.
    // ⛔⛔ **`unlocked` is exempt, and finding out why is the reason this test
    // is worth having.** The first cut banned the stem and immediately caught
    // `kit.randomize: "Re-roll every unlocked pad from this folder"` — which is
    // correct copy. This product has *locks*: TASK-044 put one on every lane and
    // every region, `L` toggles them, and "unlocked" is the state of one. The
    // guide is banning the startup verb — *unlock your creative potential* — not
    // the adjective for a padlock the producer can see on screen.
    //
    // ⚠ So the rule is the verb and its gerund, never the past participle. A
    // hypothetical "unlocked potential" would slip through, and that is the
    // trade: a gate that fires on real copy gets switched off, and one nobody
    // switches off catches the way this actually goes wrong.
    const banned = [
      /\bleverag(e|es|ed|ing)\b/,
      /\bunlock(s|ing)?\b/,
      /\bempower(s|ed|ing)?\b/,
      /\bsolutions?\b/,
      /\bai[ -]powered\b/,
    ];
    const bad: string[] = [];
    for (const [key, value] of en) {
      const text = value.toLowerCase();
      if (banned.some((word) => word.test(text))) bad.push(`${key}: "${value}"`);
    }
    expect(bad, `the voice guide forbids these words:\n  ${bad.join('\n  ')}`).toEqual([]);
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
    const missing = new Set<string>();

    for (const file of sourceFiles()) {
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
  const TEMPLATED_GROUPS = [
    ['settings', CATEGORIES],
    ['tabs', GENERATOR_TABS],
    ['sections', SECTIONS],
    ['song.kind', SECTION_KINDS],
    ['theme', THEME_PREFERENCES],
    ['theme.short', THEME_PREFERENCES],
    // ⚠ `ALL_LANES`, not the drum grid's `LANE_ORDER`. It is a superset — the
    // grid draws the nine percussion lanes and the KIT panel draws all
    // thirteen, so checking the smaller list would let `lanes.chords` go
    // missing and render as its own key in the panel (TASK-131B).
    ['lanes', ALL_LANES],
    ['scales', SCALES],
    // The roster's tier badges (TASK-047). `ArtistPane` templates the tier
    // straight into the key, so a tier with no string renders as
    // `roster.inherited` at the producer — which is how `inherited`, the one
    // nothing in the UI had ever shown, was found.
    ['roster', TIERS],
    // The shortcuts panel (TASK-131I). Driven off the panel's own catalog
    // rather than a list restated here, so a shortcut added to the app without
    // a string fails this instead of rendering its own key at the producer.
    ['shortcuts.groups', SHORTCUT_GROUPS.map((group) => group.id)],
    ['shortcuts.keys', SHORTCUT_GROUPS.flatMap((group) => group.items.map((item) => item.id))],
    // Why a layered `.mid` was routed where it was (TASK-058D). `MidiPreview`
    // templates the reason straight into the key, and the reasons are the
    // engine's own enum — so a variant added in Rust with no string here
    // renders as `explorer.splitReason.innerVoice` at the producer, which is
    // exactly the failure the tier badges above were found by.
    ['explorer.splitReason', SPLIT_REASONS],
  ] as const;

  it.each(TEMPLATED_GROUPS)(
    'defines a %s entry for every value the UI iterates',
    (group, values) => {
      const missing = values.filter((value) => !en.has(`${group}.${value}`));
      expect(missing).toEqual([]);
    },
  );

  it('registers every templated key prefix the source actually uses', () => {
    // The registry above is hand-maintained, which makes it exactly the kind of
    // thing that silently falls behind: a new `t(`foo.${x}`)` group is checked
    // by nothing until someone remembers to add a row. This finds the prefixes
    // in the source and demands each one be registered — so forgetting fails
    // here rather than shipping a key rendered as literal text.
    const registered = new Set<string>(TEMPLATED_GROUPS.map(([group]) => group));
    const found = new Set<string>();
    for (const file of sourceFiles()) {
      for (const [, prefix] of readFileSync(file, 'utf8').matchAll(
        /\bt\(\s*`([a-zA-Z0-9_.]+)\.\$\{/g,
      )) {
        found.add(prefix);
      }
    }
    expect([...found].filter((p) => !registered.has(p)).sort()).toEqual([]);
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
