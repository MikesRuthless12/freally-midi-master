/**
 * WCAG 2.1 relative luminance and contrast ratio — the one implementation.
 *
 * ⛔ **Three copies of this had already drifted at the threshold.** The token
 * gate used sRGB's `0.04045`; the piano roll's outline picker and the roll's own
 * design spec each grew a copy using `0.03928`, which is WCAG 2.0's older
 * figure. So the code that *chose* a colour for contrast and the test that
 * *checked* it were computing different numbers — the exact way an accessibility
 * gate passes while the thing it guards is wrong.
 *
 * `0.04045` is the current value, from WCAG 2.1's relative-luminance definition.
 */

const CHANNEL_THRESHOLD = 0.04045;

function channel(value: number): number {
  const c = value / 255;
  return c <= CHANNEL_THRESHOLD ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/**
 * Parse `#rrggbb`, `#rgb` or `rgb(r, g, b)` into channels.
 *
 * Both spellings, because the two callers get colours from different places: a
 * stylesheet is parsed as hex, and `getComputedStyle` always resolves to
 * `rgb(...)` whatever the author wrote.
 */
function channelsOf(color: string): [number, number, number] {
  const text = color.trim();

  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(text);
  if (hex) {
    const digits = hex[1].length === 3 ? [...hex[1]].map((d) => d + d).join('') : hex[1];
    const n = parseInt(digits, 16);
    return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
  }

  const parts = text.match(/-?\d+(\.\d+)?/g);
  if (parts === null || parts.length < 3) {
    throw new Error(`not a colour this understands: ${color}`);
  }
  return [Number(parts[0]), Number(parts[1]), Number(parts[2])];
}

export function luminance(color: string): number {
  const [r, g, b] = channelsOf(color);
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** The WCAG contrast ratio between two colours, 1–21. Order does not matter. */
export function contrastRatio(a: string, b: string): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
}
