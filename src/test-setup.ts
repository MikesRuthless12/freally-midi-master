import { initI18n } from './i18n';

/**
 * Node 26 defines its own `localStorage` global that is unavailable unless the
 * runtime is started with `--localstorage-file`, and it shadows the one jsdom
 * would otherwise provide. The result is that `window.localStorage` is
 * `undefined` under test while being perfectly normal in the WebView the app
 * actually ships in.
 *
 * Rather than weaken the tests to accommodate that, install a real in-memory
 * Storage so persistence behaviour can be asserted properly.
 */
class MemoryStorage implements Storage {
  #map = new Map<string, string>();

  get length(): number {
    return this.#map.size;
  }

  clear(): void {
    this.#map.clear();
  }

  getItem(key: string): string | null {
    return this.#map.has(key) ? this.#map.get(key)! : null;
  }

  key(index: number): string | null {
    return [...this.#map.keys()][index] ?? null;
  }

  removeItem(key: string): void {
    this.#map.delete(key);
  }

  setItem(key: string, value: string): void {
    this.#map.set(key, String(value));
  }
}

// Setup files run for every environment, including the `node` one used by the
// token-contrast test, which has no window to patch.
if (typeof window !== 'undefined') {
  Object.defineProperty(window, 'localStorage', {
    value: new MemoryStorage(),
    configurable: true,
    writable: true,
  });
}

/**
 * Initialise i18n once for the whole suite.
 *
 * Without it `t('rails.searchLabel')` returns the raw key, so every component
 * test asserts against "rails.searchLabel" instead of real text — which would
 * pass just as well if the catalog were empty. Tests should see what a user
 * sees, so they run against the real English catalog.
 */
if (typeof window !== 'undefined') {
  initI18n();
}
