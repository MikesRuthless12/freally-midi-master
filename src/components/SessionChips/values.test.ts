import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { KEY_NAMES, SCALES, keyName, prettyKey } from './values';

const here = dirname(fileURLToPath(import.meta.url));

describe('session values', () => {
  it('offers exactly the scales the engine can generate in', () => {
    // A TypeScript union has no runtime form, so `SCALES` is hand-written and
    // would silently fall behind the Rust enum. Reading the generated union
    // back out of the bindings is the only thing that notices: add a scale in
    // `engine::pattern::Scale`, run the tests, and this fails until the chip
    // can offer it.
    const bindings = readFileSync(join(here, '..', '..', 'lib', 'ipc-types.ts'), 'utf8');
    const declared = /export type Scale = ([^;]+);/.exec(bindings);
    expect(declared, 'ipc-types.ts must declare a Scale union').not.toBeNull();

    // ⛔ **Digits are part of a scale name.** This read `[a-z_]+`, which was
    // true of all twelve original scales and stopped being true the moment the
    // set grew to forty-one: `ionian_sharp5`, `dorian_sharp4`,
    // `locrian_natural6` and the five `messiaen_mode*` all carry one. The
    // scraper silently truncated them to `ionian_sharp`, so the gate compared a
    // mangled list against a correct one and failed while both sides were in
    // fact in step — a gate that cannot read its own input is worse than none,
    // because it fails in a way that invites editing the *data* to match.
    const generated = [...declared![1].matchAll(/"([a-z0-9_]+)"/g)].map((m) => m[1]);
    expect([...SCALES].sort()).toEqual([...generated].sort());
  });

  it('names all twelve pitch classes, starting at C', () => {
    expect(KEY_NAMES).toHaveLength(12);
    expect(keyName(0)).toBe('C');
    expect(keyName(6)).toBe('F♯');
    expect(keyName(11)).toBe('B');
  });

  it('has no name for something that is not a pitch class', () => {
    // `null` renders as an em dash; `undefined` would render as the string
    // "undefined" in a chip.
    expect(keyName(12)).toBeNull();
    expect(keyName(-1)).toBeNull();
    expect(keyName(null)).toBeNull();
  });

  it('respells an authored key the way the app spells it', () => {
    // The models author ASCII. A chip showing "F#" beside a pinned "F♯" reads
    // as two different notes.
    expect(prettyKey('F#')).toBe('F♯');
    expect(prettyKey('Bb')).toBe('B♭');
    expect(prettyKey('C')).toBe('C');
  });
});
