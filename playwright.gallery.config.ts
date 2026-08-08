import config from './playwright.config';

/**
 * The gallery, which the gate deliberately skips.
 *
 * ⚠ **Its own config purely to undo one line.** `playwright.config.ts` sets
 * `testIgnore` so nineteen full page loads do not ride on every gate run across
 * three OSes; this is the same configuration with that exclusion lifted, so
 * there is no second copy of the browser, viewport or dev-server setup to drift
 * from the first.
 */
export default { ...config, testIgnore: undefined };
