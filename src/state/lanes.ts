import type { Lane } from '../lib/ipc-types';

/**
 * Every lane the plugin has, in the order the KIT panel shows them.
 *
 * ⛔ **A leaf module on purpose: it imports nothing but a type.** This lived in
 * `state/kit.ts`, which reaches `state/session.ts` and from there the i18n
 * catalogs — and those use `import.meta.glob`, which only Vite understands. So
 * a Playwright spec could not import the list at all, and
 * `e2e/kit-panel.spec.ts` asserted a hardcoded `13` instead. TASK-140 took the
 * engine to 21 lanes and the spec was still checking 13.
 *
 * ⚠ **A number typed into a test is the same defect the KIT panel itself had**
 * before TASK-136, where a list written in the UI stopped matching the kit that
 * was actually loaded. One list, importable from anywhere, is the fix for both.
 *
 * ⚠ **Not what the panel iterates.** It draws whatever `kit_state` answers
 * with, so the plugin stays the one authority on which lanes exist. This is
 * here for the things that cannot ask the plugin: the browser mock's fixture,
 * `locales.test.ts`, and the end-to-end specs. Mirrors `shared::ALL_LANES`.
 *
 * ⛔⛔ **"Mirrors" was a comment, and it went stale at 21 of 37.** TASK-043A
 * took the engine to 37 lanes and this list stayed at TASK-140's 21 — so the
 * two gates that count against it went on passing over sixteen lanes that did
 * not exist here: `locales.test.ts` would not have noticed a missing name in
 * any of the 18 catalogs for any new lane, and `kit-panel.spec.ts` would have
 * stayed green with all sixteen new pads dropped. Two green readouts over a
 * list 43% short is the worst shape a gate can take. `lanes.test.ts` now reads
 * the **generated** `Lane` union out of `ipc-types.ts` and fails on any
 * difference, so the comment is enforced rather than promised.
 */
export const ALL_LANES: Lane[] = [
  'kick',
  'subKick',
  'snare',
  'offSnare',
  'ghostSnare',
  'clap',
  'closedHat',
  'openHat',
  'pedalHat',
  'ride',
  'rideBell',
  'crash',
  'tom',
  'tomHigh',
  'tomLow',
  'rim',
  'snap',
  'perc',
  'perc2',
  'shaker',
  'tambourine',
  'cowbell',
  'clave',
  'conga',
  'bongo',
  'timbale',
  'triangle',
  'woodblock',
  'riser',
  'impact',
  'reverse',
  'sub',
  'subLow',
  'melody',
  'counter',
  'bass',
  'chords',
];
