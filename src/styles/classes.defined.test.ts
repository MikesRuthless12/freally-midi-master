import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';

/**
 * Every shared `btn-*` class a component reaches for has to exist (TASK-093).
 *
 * ⛔⛔ **This defect has now shipped twice, and both times it looked like a
 * design nobody finished rather than a bug.** `.btn-generate--secondary` was
 * "applied since TASK-120 and defined nowhere until 2026-08-15", so *Generate
 * all* rendered as a second full-weight primary beside Generate. Then
 * `btn-secondary` on the pattern library's Save button matched nothing at all,
 * so it drew as bare text — Mike, 2026-08-19: *"the save button over here needs
 * to be an actual button, not just a label."* It was a `<button>` the whole
 * time. The markup was never wrong; the class was.
 *
 * ⚠ **It fails silently by construction**, which is why this is a test rather
 * than something review would catch: an unmatched class name is not an error in
 * CSS, in TypeScript, or in the build. `tokens.defined.test.ts` exists for the
 * same reason one level down — it catches a `var()` naming a token nobody
 * defines; this catches a class naming a rule nobody wrote.
 *
 * ▶ **Scoped to `btn-*` deliberately, and not widened to every class.** The
 * component-scoped BEM names (`patterns__savebtn`) live beside their own
 * stylesheet and a missing one is visible the moment the component is opened.
 * The `btn-*` family is the shared vocabulary — declared in `layout.css`, used
 * across a dozen components that never import it — which is exactly the shape
 * that goes unnoticed, and it is the only family that has actually failed.
 */

const src = join(dirname(fileURLToPath(import.meta.url)), '..');

/** Every file under `src/`, as a path. */
function sources(pattern: RegExp): string[] {
  return (
    readdirSync(src, { recursive: true, encoding: 'utf8' })
      .filter((name) => pattern.test(name))
      .map((name) => join(src, name))
      // ⚠ A recursive read lists directories too, and `readFileSync` throws
      // `EISDIR` on one — `tokens.defined.test.ts` carries the same line.
      .filter((path) => statSync(path).isFile())
  );
}

/** Every `btn-*` class any stylesheet defines, however it is qualified. */
function defined(): Set<string> {
  const names = new Set<string>();
  for (const sheet of sources(/\.css$/)) {
    const body = readFileSync(sheet, 'utf8');
    for (const [, name] of body.matchAll(/\.(btn-[a-zA-Z0-9_-]+)/g)) names.add(name);
  }
  return names;
}

/** Every `btn-*` class a component actually puts on an element. */
function used(): Map<string, string[]> {
  const where = new Map<string, string[]>();
  for (const file of sources(/\.tsx?$/)) {
    if (/\.(test|spec)\./.test(file)) continue;
    const body = readFileSync(file, 'utf8');
    // Only inside a className, so a `btn-` mentioned in prose is not a usage.
    for (const [, value] of body.matchAll(/className=(?:"([^"]*)"|\{`([^`]*)`\})/g)) {
      for (const [, name] of (value ?? '').matchAll(/\b(btn-[a-zA-Z0-9_-]+)/g)) {
        where.set(name, [...(where.get(name) ?? []), file.slice(src.length + 1)]);
      }
    }
  }
  return where;
}

it('defines every shared button class a component uses', () => {
  const have = defined();
  const missing = [...used().entries()]
    .filter(([name]) => !have.has(name))
    .map(([name, files]) => `${name} — used in ${[...new Set(files)].join(', ')}`);

  expect(
    missing,
    'these button classes are applied to an element but no stylesheet defines them, so they ' +
      'render with no styling at all and look like plain text. Add the rule, or use one that ' +
      'exists.',
  ).toEqual([]);
});
