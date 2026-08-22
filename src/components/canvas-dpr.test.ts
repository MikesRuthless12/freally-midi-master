import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';

/**
 * Every canvas that paints to the screen scales by `devicePixelRatio` (TASK-097).
 *
 * ⛔⛔ **A canvas sized in CSS pixels is BLURRY on every machine a producer is
 * likely to own.** The plugin editor is resizable inside a host, Windows ships
 * at 125% and 150% by default on laptops, and macOS is 2× everywhere — so the
 * unscaled case is the *unusual* one. `PianoRoll`'s own header records what it
 * costs: a hairline at a fractional CSS pixel lands between device pixels and
 * draws as a smear.
 *
 * ▶ **A source rule rather than a rendering assertion, and deliberately.** jsdom
 * has no layout: `clientWidth` is 0 and `getContext('2d')` draws nothing, so a
 * test that tried to measure a backing store could only ever compare zero to
 * zero. What is checkable is the thing that actually regresses — somebody adds a
 * canvas and forgets. `locales.test.ts` and `lanes.ts` use the same shape for the
 * same reason.
 *
 * ⚠ **Exemptions are named here with their reason, never inferred.** A silent
 * allowance is how this rule stops meaning anything.
 */

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/**
 * Canvases that are not painted to the screen, and why.
 *
 * ⛔ Both of these would be made **wrong** by scaling, not merely unnecessary —
 * which is why they are exemptions rather than oversights.
 */
const NOT_ON_SCREEN: Record<string, string> = {
  'components/Kit/dragPreview.ts':
    'a fixed-size RGBA bitmap handed to the OS drag helper, not a screen surface — ' +
    '`drag::Preview` expects exactly PREVIEW_WIDTH × PREVIEW_HEIGHT, so scaling it would corrupt the payload',
  'components/SessionChips/SessionChips.tsx':
    'a text-measurement ruler for caret placement — `measureText` is in CSS pixels ' +
    'by definition and is never drawn',
};

/**
 * Every non-test source file under `src/`.
 *
 * ⚠ **The house idiom** — `src/i18n/locales.test.ts` and
 * `src/styles/tokens.defined.test.ts` both use this one-line recursive read. The
 * hand-rolled walk it replaced called `statSync` on all 740 entries, ~550 of them
 * binary assets that can never match, and ran the whole walk once per `it()`.
 *
 * ⛔ **These are the ids, kept rather than recomputed.** `readdirSync` already
 * yields names relative to `root`; mapping them to absolute and converting back
 * with `relative()` wrote the separator normalization twice, in two `it()`
 * blocks that have to agree. If they ever drifted, the exemption *lookup* and
 * the exemption-*exists* check would disagree — which is the exact hole the
 * second test was added to close.
 */
const SOURCES: string[] = readdirSync(root, { recursive: true, encoding: 'utf8' })
  .filter((name) => /\.tsx?$/.test(name) && !/\.(test|spec)\./.test(name))
  .map((name) => name.split('\\').join('/'))
  // ⚠ **Not decoration — `readFileSync` throws `EISDIR` without it**, and
  // `locales.test.ts:63` carries the same line for the same reason. A recursive
  // read lists directories too, and `src/` has ones whose names end in the
  // letters the extension test looks for.
  .filter((id) => statSync(join(root, id)).isFile());

it('scales every on-screen canvas by devicePixelRatio', () => {
  const missing: string[] = [];

  for (const id of SOURCES) {
    const body = readFileSync(join(root, id), 'utf8');
    if (!/getContext\(\s*['"]2d['"]\s*\)/.test(body)) continue;

    if (id in NOT_ON_SCREEN) continue;
    if (body.includes('devicePixelRatio')) continue;

    missing.push(id);
  }

  expect(
    missing,
    'these files draw on a canvas without scaling it by devicePixelRatio — the result is ' +
      'blurry at 125%, 150% and on every Mac. Add the scaling, or add the file to ' +
      'NOT_ON_SCREEN with the reason it is not a screen surface.',
  ).toEqual([]);
});

it('keeps every exemption pointing at a file that still exists', () => {
  // ⚠ An exemption for a deleted or renamed file is a hole that stays open with
  // nothing to show for it — and the next canvas added at that path inherits it
  // silently, which is exactly the failure the rule exists to prevent.
  const all = new Set(SOURCES);
  for (const id of Object.keys(NOT_ON_SCREEN)) {
    expect(all.has(id), `NOT_ON_SCREEN names ${id}, which is not in src/`).toBe(true);
  }
});
