import { expect, test } from '@playwright/test';

import { generate } from './app';

/**
 * The offline promise, enforced instead of stated (TASK-096).
 *
 * ⛔⛔ **This product's headline claim is that it never touches the network**, and
 * the About pane says so to the producer's face — *"engineered, not trained —
 * zero AI"*, no telemetry, everything local. A promise printed in the UI should
 * be a gate, not a statement.
 *
 * ⛔ **This REPLACES the weaker check that lived in `phase-gate.spec.ts`**, which
 * was `test('the UI makes no network requests of its own')` under *"Phase gate —
 * offline and AI-free"*. Keeping both would have been two gates claiming the same
 * thing with the weaker one left to rot. What it did differently, and why each
 * change is the stricter reading:
 * - It matched `url.startsWith('http://localhost:1420')`, a **scheme-and-port
 *   prefix**. That is not the claim — see [`LOOPBACK`].
 * - It intercepted with `context.route('**\/*')`, which is a request handler
 *   rather than an observer. `page.on('request')` cannot alter what it watches.
 * - Its journey was a reload and one tab click, so nothing it exercised ever
 *   generated. The network call worth fearing is on the path the product exists
 *   for.
 * - It watched neither websockets nor `script[src]`.
 *
 * ⚠ **`scripts/check-denylist.mjs` is the other half and is not replaced.** It
 * reads the **dependency graph** — it proves no HTTP client is linked, and it
 * cannot prove the code we wrote ourselves never calls `fetch`. This proves the
 * second and not the first.
 *
 * ▶ **A runtime assertion rather than a grep**, because that is the claim being
 * made. The page is driven through a real journey and every request the browser
 * attempts is recorded; anything leaving the dev server's own origin fails the
 * test, whoever issued it and however it was spelled.
 *
 * ⚠ **What this cannot see, stated plainly so nobody reads more into a green
 * run than is there:** the Rust half. A socket opened by the plugin process
 * never appears to the browser. `check-denylist.mjs` is what covers that side,
 * and the two together are the whole claim — neither alone is.
 */

/**
 * Hosts that are this machine.
 *
 * ⛔ **Compared by HOST, not by origin string, and the difference is the whole
 * accuracy of the test.** The claim being enforced is *"nothing leaves the
 * machine"* — not *"every URL starts with the same seven characters"*. A
 * scheme-prefix check failed on Vite's own HMR socket, `ws://localhost:1420`,
 * which is the same machine over a different scheme; reporting that as a
 * telemetry leak would have taught the next person to ignore this gate.
 *
 * ⚠ **The HMR socket is a DEV-SERVER artifact and does not ship.** In the
 * plugin the page is served over the webview's custom protocol with no dev
 * server behind it, so nothing here is being waved through in the built product.
 */
const LOOPBACK = ['localhost', '127.0.0.1', '[::1]'];

/**
 * Did this request leave the machine?
 *
 * ⚠ Kept deliberately narrow. Every exemption is a hole in the assertion, so a
 * new one needs a reason rather than a convenience.
 */
function isOffBox(url: string): boolean {
  // `data:` and `blob:` never leave the process — the drag preview is a data
  // URI — and neither parses as having a host.
  if (url.startsWith('data:') || url.startsWith('blob:')) return false;
  try {
    return !LOOPBACK.includes(new URL(url).hostname);
  } catch {
    // An unparseable URL is not something to wave through.
    return true;
  }
}

test('the app opens no connection off the machine, through a whole journey', async ({
  page,
}) => {
  const offBox: string[] = [];

  // ⛔ **`request`, not `response`.** A blocked or failed connection still had a
  // socket opened for it, and the promise is that none is attempted — a check
  // that only watched replies would pass on a telemetry endpoint that happened
  // to be unreachable from CI.
  page.on('request', (request) => {
    if (isOffBox(request.url())) offBox.push(`${request.method()} ${request.url()}`);
  });
  // WebSockets do not surface as requests, so they are watched separately.
  page.on('websocket', (socket) => {
    if (isOffBox(socket.url())) offBox.push(`WS ${socket.url()}`);
  });

  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  // ⛔ **A second claim, checked on the page this test already loaded.** A
  // CDN-hosted `<script>` is an off-box *dependency* rather than an off-box
  // *call*, so it deserves its own assertion — but paying a second browser
  // context and a second full app boot to make it would be a whole extra page
  // load per e2e run, on every OS in the matrix.
  const scripts = await page.evaluate(() =>
    [...document.querySelectorAll('script[src]')].map((tag) => (tag as HTMLScriptElement).src),
  );
  for (const src of scripts) {
    expect(isOffBox(src), `a script is served from off the machine: ${src}`).toBe(false);
  }

  // The journey the product exists for.
  await generate(page, 'UK Drill');

  expect(
    offBox,
    'the product promises it never touches the network, and these requests left the machine',
  ).toEqual([]);
});
