import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import { ALL_LANES } from './lanes';

/**
 * The gate that makes "Mirrors `shared::ALL_LANES`" true rather than hopeful.
 *
 * ⛔⛔ **The comment said it and nothing checked it, so it drifted to 21 of
 * 37.** Sixteen lanes shipped without ever entering this list, and the two
 * things that count against it — `locales.test.ts`, which checks every lane has
 * a name in all 18 catalogs, and `e2e/kit-panel.spec.ts`, which counts pads —
 * both went on passing. A gate that reports success over a list 43% short is
 * worse than no gate: it is the reason nobody looked.
 *
 * ⚠ **Read out of the generated bindings, not out of the Rust.** `ipc-types.ts`
 * is written by `ts-rs` from `engine::pattern::Lane`, and `npm run ci:local`
 * already fails if it is stale — so it is the one artifact in the tree that
 * cannot silently disagree with the engine. Parsing a `.rs` file from a vitest
 * would just be a third list to keep in step.
 */
function generatedLanes(): string[] {
  const bindings = readFileSync('src/lib/ipc-types.ts', 'utf8');
  const line = bindings.split('\n').find((text) => text.startsWith('export type Lane ='));
  if (line === undefined)
    throw new Error('ipc-types.ts has no `Lane` union to compare against');
  return [...line.matchAll(/"([a-zA-Z0-9]+)"/g)].map((match) => match[1]);
}

describe('the lane mirror', () => {
  it('holds every lane the engine has, in the engine’s order', () => {
    // Order as well as membership: `ALL_LANES` is what the KIT panel and the
    // mock's fixture enumerate, so a list with the right names in the wrong
    // order draws the pads in an order the plugin never uses.
    expect(ALL_LANES).toEqual(generatedLanes());
  });

  it('has no duplicates', () => {
    // A repeat would make `locales.test.ts` and the pad count both read high
    // while still missing whatever it displaced.
    expect(new Set(ALL_LANES).size).toBe(ALL_LANES.length);
  });
});
