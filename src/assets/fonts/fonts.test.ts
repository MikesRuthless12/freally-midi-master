/**
 * Every language the UI can switch to must have glyphs to render with.
 *
 * The failure this prevents is tofu — the empty rectangle a browser draws when
 * no font in the stack covers a character. It is invisible in development,
 * because an English-speaking developer never types Japanese into the UI, and it
 * is total when it happens: a Japanese user sees a screen of boxes.
 *
 * So this asserts the property directly, over the real vendored CSS: for every
 * canonical locale, every character of a representative sample resolves to some
 * bundled @font-face, and the file that face names is actually on disk. It is a
 * data test, not a rendering test — jsdom has no font engine — but the thing it
 * checks is the thing that goes wrong.
 *
 * Regenerate the fonts with `node scripts/vendor-fonts.mjs`.
 */

import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

/**
 * The Havoc canonical 18 (locale_overhaul_plan.md), with a sample that exercises
 * the script rather than the language — what matters here is which codepoints
 * need covering, not whether the words are idiomatic.
 */
const LOCALE_SAMPLES: Record<string, string> = {
  en: 'Generate MIDI',
  ar: 'إنشاء ملف ميدي',
  de: 'MIDI erzeugen — Größe',
  es: 'Generar MIDI — canción',
  fr: 'Générer du MIDI — clé',
  hi: 'मिडी बनाएँ',
  id: 'Hasilkan MIDI',
  it: 'Genera MIDI — però',
  ja: 'MIDIを生成する',
  ko: 'MIDI 생성하기',
  nl: 'MIDI genereren',
  pl: 'Generuj MIDI — źdźbło',
  'pt-BR': 'Gerar MIDI — configurações',
  ru: 'Создать MIDI',
  tr: 'MIDI oluştur — ğüşiöç',
  uk: 'Створити MIDI — ї є ґ',
  vi: 'Tạo MIDI — nhạc',
  'zh-CN': '生成 MIDI 文件',
};

/** Characters every locale shows regardless of language: the UI's own chrome. */
const ALWAYS_PRESENT = '0123456789.,:;()[]%/–—←→↑↓×✓•…“”‘’';

type Face = { family: string; file: string; ranges: Array<[number, number]> };

function parseUnicodeRange(value: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  for (const raw of value.split(',')) {
    const token = raw.trim().replace(/^U\+/i, '');
    if (!token) continue;
    if (token.includes('-')) {
      const [lo, hi] = token.split('-');
      ranges.push([parseInt(lo, 16), parseInt(hi, 16)]);
    } else if (token.includes('?')) {
      // `U+04??` — a wildcard span.
      ranges.push([
        parseInt(token.replace(/\?/g, '0'), 16),
        parseInt(token.replace(/\?/g, 'F'), 16),
      ]);
    } else {
      const point = parseInt(token, 16);
      ranges.push([point, point]);
    }
  }
  return ranges;
}

function loadFaces(): Face[] {
  const faces: Face[] = [];
  for (const sheet of ['fonts.css', 'fonts-scripts.css']) {
    const path = join(here, sheet);
    expect(existsSync(path), `${sheet} is missing — run scripts/vendor-fonts.mjs`).toBe(true);
    const css = readFileSync(path, 'utf8');

    for (const [, block] of css.matchAll(/@font-face\s*\{([^}]*)\}/g)) {
      const family = /font-family:\s*'([^']+)'/.exec(block)?.[1] ?? '';
      const file = /url\('\.\/([^']+)'\)/.exec(block)?.[1] ?? '';
      const range = /unicode-range:\s*([^;]+);/.exec(block)?.[1] ?? '';
      faces.push({ family, file, ranges: parseUnicodeRange(range) });
    }
  }
  return faces;
}

const faces = loadFaces();

/** Every codepoint the bundled fonts can draw. */
function covers(codePoint: number): boolean {
  return faces.some((face) =>
    face.ranges.some(([lo, hi]) => codePoint >= lo && codePoint <= hi),
  );
}

function uncovered(text: string): string[] {
  const missing = new Set<string>();
  for (const char of text) {
    const point = char.codePointAt(0);
    // A plain space is never in a unicode-range and never needs to be.
    if (point === undefined || char === ' ') continue;
    if (!covers(point)) missing.add(`${char} (U+${point.toString(16).toUpperCase()})`);
  }
  return [...missing];
}

describe('bundled Noto fonts', () => {
  it('ships a usable number of faces', () => {
    // A parser regression in the vendoring script once produced four faces for
    // Chinese instead of a hundred, which would have shipped as tofu.
    expect(faces.length).toBeGreaterThan(400);
  });

  it('names a real file for every face', () => {
    const orphans = faces.filter((f) => !f.file || !existsSync(join(here, f.file)));
    expect(orphans.map((f) => `${f.family}: ${f.file || '(no url)'}`)).toEqual([]);
  });

  it.each(Object.entries(LOCALE_SAMPLES))('renders %s without tofu', (_locale, sample) => {
    expect(uncovered(sample)).toEqual([]);
  });

  it('renders the UI chrome shared by every locale', () => {
    expect(uncovered(ALWAYS_PRESENT)).toEqual([]);
  });

  it('actually detects a script that is not bundled', () => {
    // Without this the suite above could be passing vacuously — a coverage
    // check that never fails is indistinguishable from one that always says
    // yes. Tibetan is deliberately not in the vendored set, so it must be
    // reported. If Tibetan is ever added, this test should start failing and
    // be pointed at whatever is still missing.
    expect(uncovered('བོད་སྐད')).not.toEqual([]);
  });

  it('keeps the deferred sheet out of the blocking one', () => {
    // The split is what keeps startup fast: ~450 KB of per-script @font-face
    // rules must not end up in the stylesheet that blocks first paint.
    const blocking = readFileSync(join(here, 'fonts.css'), 'utf8');
    const declared = [...blocking.matchAll(/@font-face\s*\{([^}]*)\}/g)].map(
      ([, block]) => /font-family:\s*'([^']+)'/.exec(block)?.[1] ?? '',
    );
    expect(declared.length).toBeLessThan(40);

    // Naming a per-script family in the fallback chain is the whole point and
    // must stay allowed; what must not be here is a *definition* of one, since
    // that is the ~450 KB the split exists to defer.
    expect(declared.filter((family) => family === 'Noto Sans SC')).toEqual([]);
    expect([...new Set(declared)].sort()).toEqual([
      'Noto Sans',
      'Noto Sans Display',
      'Noto Sans Mono',
    ]);
  });

  it('declares a fallback chain covering every bundled script family', () => {
    const blocking = readFileSync(join(here, 'fonts.css'), 'utf8');
    const chain = /--font-noto-fallback:([^;]+);/.exec(blocking)?.[1] ?? '';
    expect(chain).not.toBe('');

    // Every family that has faces must be reachable from the stack, or its
    // glyphs are on disk and unusable.
    const deferred = new Set(
      faces.map((f) => f.family).filter((name) => name.startsWith('Noto Sans ')),
    );
    const unreachable = [...deferred].filter(
      (name) =>
        !chain.includes(`'${name}'`) && !['Noto Sans Display', 'Noto Sans Mono'].includes(name), // named directly by tokens.css
    );
    expect(unreachable).toEqual([]);
  });

  it('ships the licence the OFL requires to travel with the fonts', () => {
    const licence = readFileSync(join(here, 'OFL-Noto.txt'), 'utf8');
    expect(licence).toContain('SIL OPEN FONT LICENSE Version 1.1');
  });
});
