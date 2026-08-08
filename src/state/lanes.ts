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
 */
export const ALL_LANES: Lane[] = [
  'kick',
  'snare',
  'offSnare',
  'clap',
  'closedHat',
  'openHat',
  'ride',
  'crash',
  'tom',
  'rim',
  'snap',
  'perc',
  'shaker',
  'tambourine',
  'cowbell',
  'woodblock',
  'sub',
  'melody',
  'counter',
  'bass',
  'chords',
];
