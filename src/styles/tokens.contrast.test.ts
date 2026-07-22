/**
 * Guards WCAG 2.1 AA contrast for BOTH themes (PRD § 7 Accessibility).
 *
 * The values are parsed out of tokens.css rather than duplicated here, so a
 * token edit that breaks contrast fails this test instead of shipping.
 *
 * @vitest-environment node
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const css = readFileSync(fileURLToPath(new URL('./tokens.css', import.meta.url)), 'utf8');

/** Every `--light-*` value, so `var(--light-bg)` can be resolved. */
function lightPalette(): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [, name, value] of css.matchAll(/(--light-[\w-]+)\s*:\s*([^;]+);/g)) {
    out[name] = value.trim();
  }
  return out;
}

/** Pull the declarations out of the first rule whose selector contains `needle`. */
function tokensOf(needle: string): Record<string, string> {
  const start = css.indexOf(needle);
  if (start === -1) throw new Error(`no rule matching ${needle} in tokens.css`);
  const open = css.indexOf('{', start);
  const close = css.indexOf('}', open);
  const body = css.slice(open + 1, close);

  const palette = lightPalette();
  const out: Record<string, string> = {};
  for (const [, name, value] of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    const raw = value.trim();
    // Resolve one level of `var(--light-x)` indirection so the assertions see
    // real colours rather than a variable reference.
    const ref = /^var\((--light-[\w-]+)\)$/.exec(raw);
    out[name] = ref ? (palette[ref[1]] ?? raw) : raw;
  }
  return out;
}

function channel(c: number): number {
  const s = c / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

function luminance(hex: string): number {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) throw new Error(`not a 6-digit hex colour: ${hex}`);
  const n = parseInt(m[1], 16);
  return (
    0.2126 * channel((n >> 16) & 0xff) +
    0.7152 * channel((n >> 8) & 0xff) +
    0.0722 * channel(n & 0xff)
  );
}

function contrast(fg: string, bg: string): number {
  const a = luminance(fg);
  const b = luminance(bg);
  const [hi, lo] = a > b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

const THEMES = {
  dark: tokensOf(":root,\n:root[data-theme='dark']"),
  light: tokensOf(":root[data-theme='light']"),
  // The OS-default light path, which is what most users get since 'system' is
  // the default preference. Previously untested — it was a second copy of the
  // palette that no assertion ever read.
  'light (prefers-color-scheme)': tokensOf(':root:not([data-theme])'),
};

const SURFACES = ['--color-bg', '--color-surface', '--color-surface-2'] as const;

describe.each(Object.entries(THEMES))('%s theme', (_name, t) => {
  // 4.5:1 — normal body text.
  describe.each(['--color-text', '--color-text-2'])('%s', (fg) => {
    it.each(SURFACES)(`is AA (4.5:1) on %s`, (bg) => {
      expect(contrast(t[fg], t[bg])).toBeGreaterThanOrEqual(4.5);
    });
  });

  // 3:1 — the muted tier is specified for large text and non-text UI only
  // (WCAG 1.4.3 large-text / 1.4.11 non-text contrast). It must never carry
  // small body copy; see the note in tokens.css.
  it.each(SURFACES)('--color-text-3 clears 3:1 on %s', (bg) => {
    expect(contrast(t['--color-text-3'], t[bg])).toBeGreaterThanOrEqual(3);
  });

  // 3:1 — interactive/graphical elements (WCAG 1.4.11).
  it('--color-primary clears 3:1 on --color-bg', () => {
    expect(contrast(t['--color-primary'], t['--color-bg'])).toBeGreaterThanOrEqual(3);
  });

  it.each(['--color-success', '--color-warning', '--color-error', '--color-info'])(
    '%s clears 3:1 on --color-surface-2',
    (fg) => {
      expect(contrast(t[fg], t['--color-surface-2'])).toBeGreaterThanOrEqual(3);
    },
  );
});

describe('theme parity', () => {
  it('defines exactly the same token names in both themes', () => {
    expect(Object.keys(THEMES.light).sort()).toEqual(Object.keys(THEMES.dark).sort());
  });

  it('the two light selectors resolve to identical colours', () => {
    // They are separate CSS rules; if they ever diverge, the OS-default path
    // and the explicit toggle would render differently.
    expect(THEMES['light (prefers-color-scheme)']).toEqual(THEMES.light);
  });
});
