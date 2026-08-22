/**
 * The crash pane's words, and its last-resort copy of them (TASK-093).
 *
 * ⚠ **A separate file so `ErrorBoundary.tsx` exports components only** — the
 * fast-refresh rule this repo lints for, and the reason is real: a module that
 * mixes a component with a constant loses hot reload for the component.
 */

export type ErrorBoundaryStrings = {
  title: string;
  body: string;
  retry: string;
  details: string;
};

/**
 * ⚠ English, and deliberately not read from the catalog.
 *
 * If i18n is what threw, these are what the producer sees. Untranslated beats
 * absent — and `locales.test.ts` cannot police a fallback that is not a catalog
 * key, so there is nothing here for it to call an untranslated copy.
 */
export const FALLBACK_STRINGS: ErrorBoundaryStrings = {
  title: 'Something in the window stopped working',
  body: 'Your project is still open in the host. Reload the panel to carry on — nothing has been written to disk.',
  retry: 'Reload the panel',
  details: 'Technical detail',
};
