import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { ACCEL, SHORTCUT_GROUPS } from './catalog';

/**
 * ⛔ **A panel that lies about the shortcuts is worse than no panel**, because
 * it is the thing a producer trusts *instead of* experimenting. The catalog is
 * a hand-written list beside the real handlers, so the only question worth
 * testing is whether it has drifted from them.
 */

const src = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

function sourceText(): string {
  return readdirSync(src, { recursive: true, encoding: 'utf8' })
    .filter((f) => /\.tsx?$/.test(f) && !/\.test\./.test(f))
    .map((f) => join(src, f))
    .filter((f) => statSync(f).isFile())
    .map((f) => readFileSync(f, 'utf8'))
    .join('\n');
}

describe('the shortcut catalog', () => {
  it('documents no shortcut twice', () => {
    // Two rows for one action is a panel that has been edited without being
    // read, and it is the first symptom of the list drifting.
    const ids = SHORTCUT_GROUPS.flatMap((group) => group.items.map((item) => item.id));
    expect(ids).toEqual([...new Set(ids)]);

    const groups = SHORTCUT_GROUPS.map((group) => group.id);
    expect(groups).toEqual([...new Set(groups)]);
  });

  it('gives every group at least one shortcut', () => {
    // An empty group draws a heading with nothing under it, which reads as a
    // feature that failed to load rather than as a group nobody filled in.
    for (const group of SHORTCUT_GROUPS) {
      expect(group.items.length, `${group.id} is empty`).toBeGreaterThan(0);
    }
  });

  it('only documents keys that some handler actually listens for', () => {
    // ⛔ **The direction that matters.** Documenting a shortcut the app does not
    // have sends a producer to press a key that does nothing and conclude the
    // build is broken. The reverse — a real shortcut left undocumented — is a
    // gap rather than a lie, and cannot be checked this way without a registry
    // the handlers do not have.
    //
    // ⚠ Compared on the *bare key*, not the rendered label: the panel writes
    // "Ctrl + A" for the reader while the handler tests `key === 'a'`.
    const text = sourceText();
    const undocumented: string[] = [];

    for (const group of SHORTCUT_GROUPS) {
      for (const item of group.items) {
        const bare = item.keys
          .replace(ACCEL, '')
          .replace(/Alt|\+|click|\s/g, '')
          .split('/')[0];
        if (bare === '') continue;

        // ⛔ **Digits are skipped, and pretending otherwise was worse than
        // skipping them.** `DrumGrid` binds the tuplets with a *range* —
        // `Number(event.key)` then `count < 2 || count > 9` — so there is no
        // `'7'` anywhere to find. Ctrl+3 and Ctrl+5 were passing only because
        // those digits happen to appear as literals elsewhere in the source,
        // which is a coincidence rather than a check: the test was reporting
        // Ctrl+7 as phantom while all three work identically. A check that is
        // right by accident is the thing this file exists to prevent.
        if (/^\d$/.test(bare)) continue;

        // Arrows and named keys are spelled in full by the handlers; single
        // letters are lower-cased there.
        const candidates =
          bare === '↑'
            ? ['ArrowUp']
            : bare === '←'
              ? ['ArrowLeft']
              : bare === 'Esc'
                ? ['Escape']
                : bare === 'Space'
                  ? [' ', 'Space']
                  : [bare, bare.toLowerCase()];

        // ⚠ Three spellings, because the handlers use all three: a quoted
        // comparison (`event.key === 'Escape'`), and a **bare object key** in
        // the arrow map (`ArrowLeft: [...]`) which quoting-only matching missed
        // — it reported the nudge shortcuts as phantom while they work.
        const listened = candidates.some(
          (key) =>
            text.includes(`'${key}'`) ||
            text.includes(`"${key}"`) ||
            new RegExp(`^\\s*${key}\\s*:`, 'm').test(text),
        );
        if (!listened) undocumented.push(`${group.id}/${item.id} (${item.keys})`);
      }
    }

    expect(undocumented).toEqual([]);
  });

  it('writes the modifier the way this platform does', () => {
    // A macOS producer reading "Ctrl" presses the wrong key and the shortcut
    // does nothing, which is indistinguishable from a bug in the app.
    expect(['⌘', 'Ctrl']).toContain(ACCEL);
  });
});
