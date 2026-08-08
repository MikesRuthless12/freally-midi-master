import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Every colour a stylesheet reaches for has to exist.
 *
 * ⛔⛔ **The whole arrangement view rendered unstyled for its entire life, and
 * nothing could see it.** `SongTimeline.css` was written against `--accent`,
 * `--line`, `--surface`, `--text`, `--text-dim`, `--line-strong` and `--danger`
 * — none of which this app defines; the tokens are `--color-primary`,
 * `--color-border`, `--color-surface`, `--color-text` and friends. A bare
 * `var(--accent)` with no fallback makes its declaration invalid at
 * computed-value time, and a `color-mix()` holding one is dropped outright, so
 * the clips had no fill and no border, the section band had no background and
 * the playhead was a transparent 2px strip.
 *
 * ⚠ **It fails silently by construction**, which is why this is a test rather
 * than something anyone was ever going to notice in review: CSS has no error to
 * raise, the build does not resolve custom properties, and the result *looks*
 * like a design nobody finished. Mike's review reached the same conclusion from
 * the other end — "a clip does not look like a clip".
 */

const src = join(dirname(fileURLToPath(import.meta.url)), '..');

/** Every stylesheet in the app. */
function stylesheets(): string[] {
  return readdirSync(src, { recursive: true, encoding: 'utf8' })
    .filter((f) => f.endsWith('.css'))
    .map((f) => join(src, f))
    .filter((f) => statSync(f).isFile());
}

/**
 * The vendored font sheets are scanned for what they *define* but not checked.
 *
 * ⚠ **They have to be in the definitions**, and leaving them out was the first
 * cut's own bug: `--font-noto-fallback` is emitted into `fonts.css` by
 * `scripts/vendor-fonts.mjs` and used by `tokens.css`, so a scan that skipped
 * them reported the token set as broken. They are not *checked* because they are
 * 546 generated `@font-face` rules and nobody edits them by hand.
 */
function isGenerated(file: string): boolean {
  return file.includes('fonts');
}

/**
 * Properties written from JavaScript rather than declared in CSS.
 *
 * ⚠ **Named individually, not pattern-matched.** A rule like "anything ending in
 * `-progress` is fine" would have waved `--line-strong` through too. Each of
 * these has exactly one writer, named beside it.
 */
const SET_AT_RUNTIME = new Set([
  // `PianoRoll`/`DrumGrid` write the marker position at frame rate.
  '--playhead',
  // `PreviewPlayer` writes the audition position (TASK-132).
  '--preview-progress',
  // `App.tsx` writes the browser rail's dragged width (TASK-132).
  '--rail-left-width',
]);

/**
 * A stylesheet with its comments removed.
 *
 * ⛔ **The first cut of this gate counted its own prose.** `SongTimeline.css`
 * carries a long comment *about* the undefined names it used to reach for —
 * including the literal text `var(--line-strong, var(--line))` — and the scan
 * read those as live declarations, so the file failed a check it passes. The
 * engine's `authored_keys.rs` records landing on exactly this and for exactly
 * this reason: **strip the comments first, or the file's explanation of a bug
 * counts as the bug.**
 */
function code(file: string): string {
  return readFileSync(file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, ' ');
}

describe('CSS custom properties', () => {
  const files = stylesheets();

  const defined = new Set<string>();
  for (const file of files) {
    for (const [, name] of code(file).matchAll(/(--[a-z0-9-]+)\s*:/g)) {
      defined.add(name);
    }
  }

  it('has a non-trivial token set to check against', () => {
    expect(defined.size).toBeGreaterThan(30);
  });

  it.each(
    files.filter((f) => !isGenerated(f)).map((f) => [f.slice(src.length + 1), f] as const),
  )('%s only reaches for properties that exist', (_name, file) => {
    // ⚠ **Only the uses with no fallback.** `var(--bg, #0b0b0d)` is a
    // deliberate default and computes fine — `Eula.css` uses that form
    // throughout, on purpose, so the licence gate renders before any theme is
    // applied. A bare `var(--accent)` is the one that silently voids its whole
    // declaration, and it is the only thing worth failing a build over.
    const used = new Set(
      [...code(file).matchAll(/var\(\s*(--[a-z0-9-]+)\s*\)/g)].map((m) => m[1]),
    );
    const missing = [...used].filter((name) => !defined.has(name) && !SET_AT_RUNTIME.has(name));
    expect(missing).toEqual([]);
  });
});
