/**
 * Every keyboard shortcut the app has, grouped the way a producer would look
 * for one (TASK-131I).
 *
 * Mike, 2026-08-06: *"a 'keyboard shortcuts' panel that hides and shows for the
 * plugin and standalone, which will allow for me to show the end user what
 * keyboard shortcuts do and what groups they belong to."*
 *
 * ⛔ **A list, not a registry, and the distinction is deliberate.** This does
 * not bind anything — the handlers live where they are used
 * (`PianoRoll/shortcuts.ts`, `DrumGrid`, `App.tsx`), which is the only place
 * that knows the surrounding state. Making this the source of truth would mean
 * threading every one of those callbacks through a central dispatcher for a
 * panel that only ever reads.
 *
 * ⚠ **So it can drift, and `shortcuts.test.ts` is what keeps it honest**: it
 * greps the real handlers and fails when a key is bound and undocumented. A
 * panel that lies about the shortcuts is worse than no panel, because it is the
 * thing a producer trusts instead of experimenting.
 */

/** The modifier as this platform writes it. */
export const ACCEL = navigator.platform.toLowerCase().includes('mac') ? '⌘' : 'Ctrl';

export type Shortcut = {
  /** The keys, already written for this platform. */
  keys: string;
  /** i18n key under `shortcuts.keys`. */
  id: string;
};

export type ShortcutGroup = {
  /** i18n key under `shortcuts.groups`. */
  id: string;
  items: Shortcut[];
};

export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    id: 'transport',
    items: [
      // ⚠ **This row exists only because the binding does.**
      // `catalog.test.ts` refused it at first — play and pause were wired to
      // the transport buttons alone, in a tool whose whole audience presses
      // Space to hear something. The honest fix was to add the shortcut rather
      // than quietly document a dead key.
      { keys: 'Space', id: 'play' },
      { keys: 'K', id: 'toggleRail' },
      { keys: '? / F1', id: 'shortcuts' },
    ],
  },
  {
    id: 'generate',
    items: [
      { keys: 'Enter', id: 'search' },
      { keys: 'R', id: 'reroll' },
    ],
  },
  {
    id: 'editing',
    items: [
      { keys: `${ACCEL} + A`, id: 'selectAll' },
      { keys: `${ACCEL} + C`, id: 'copy' },
      { keys: `${ACCEL} + X`, id: 'cut' },
      { keys: `${ACCEL} + V`, id: 'paste' },
      { keys: `${ACCEL} + Z`, id: 'undo' },
      { keys: `${ACCEL} + D`, id: 'duplicate' },
      { keys: 'Delete', id: 'delete' },
      { keys: 'Esc', id: 'deselect' },
    ],
  },
  {
    id: 'notes',
    items: [
      { keys: '↑ / ↓', id: 'transpose' },
      { keys: `${ACCEL} + ↑ / ↓`, id: 'transposeOctave' },
      { keys: '← / →', id: 'nudge' },
      { keys: '*', id: 'stretch' },
      { keys: '/', id: 'compress' },
    ],
  },
  {
    id: 'drums',
    items: [
      { keys: `${ACCEL} + 3`, id: 'triplet' },
      { keys: `${ACCEL} + 5`, id: 'quintuplet' },
      { keys: `${ACCEL} + 7`, id: 'septuplet' },
      { keys: 'Alt + click', id: 'cloneBar' },
    ],
  },
  {
    id: 'browser',
    items: [
      // ⚠ **The same keys as `notes/nudge`, and that is not a clash.** The
      // browser's handler is bound on its own panel rather than on `window`
      // precisely so the piano roll keeps ← and → for nudging — see
      // `ExplorerPanel`. Two rows for two scopes is what the panel has to say,
      // because a producer reading one row would not know which one they get.
      { keys: '← / →', id: 'previewDirection' },
    ],
  },
];
