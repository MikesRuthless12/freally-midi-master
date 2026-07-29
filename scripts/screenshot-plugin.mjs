#!/usr/bin/env node
/**
 * Launch the plugin's standalone build and photograph **its own window**.
 *
 * This is TASK-P12's gate, and the reason it exists is that a Linux editor
 * which compiles proves nothing at all. The whole risk of that task is a window
 * that opens and stays empty: GTK never started, the webview was never built,
 * the page was never fetched. Every one of those failures compiles cleanly, and
 * `cargo build` on the Ubuntu runner would have gone green through all of them.
 *
 * So the assertion is not "it built" and not even "a window exists" — it is
 * **the window has a picture in it**. `identify -format %k` counts distinct
 * colours: a window that opened but never rendered is one flat rectangle of the
 * app's own background, and the UI is hundreds of colours. That is the
 * difference between the two outcomes this gate has to tell apart, and it is
 * the only one visible from outside the process.
 *
 * Linux only, deliberately. Windows and macOS are photographed by
 * `screenshot-app.mjs`, which drives the Tauri shell; when that shell goes, this
 * script grows those platforms and that one is deleted.
 *
 * Usage: node scripts/screenshot-plugin.mjs <output.png> [path/to/standalone]
 */

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

if (process.platform !== 'linux') {
  console.error('screenshot-plugin.mjs is Linux-only; see the module comment.');
  process.exit(1);
}

const output = resolve(process.argv[2] ?? 'screenshots/linux-plugin.png');
const binary = resolve(
  process.argv[3] ?? process.env.FREALLY_STANDALONE ?? 'target/release/standalone',
);
mkdirSync(dirname(output), { recursive: true });

/** `Plugin::NAME`, which nih-plug's standalone uses as the window title. */
const WINDOW_TITLE = 'Freally MIDI Master';

/** WebKitGTK on a software rasteriser is slow to first paint. */
const WINDOW_TIMEOUT_MS = 3 * 60 * 1000;
const PAINT_MS = 25_000;

/**
 * How many distinct colours mean "rendered".
 *
 * A blank window is 1 — the background this plugin sets deliberately, so that a
 * slow first paint is the app's colour rather than a white flash. Anti-aliasing
 * and the window border can lift that to a handful, so the bar sits well above
 * the noise and far below what any real frame of the UI produces (thousands).
 */
const MIN_COLOURS = 24;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function die(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

function sh(cmd, args) {
  return spawnSync(cmd, args, { encoding: 'utf8' });
}

function windowId() {
  const found = sh('xdotool', ['search', '--name', WINDOW_TITLE]);
  if (found.status !== 0) return null;
  return (found.stdout ?? '').trim().split('\n').filter(Boolean).pop() ?? null;
}

if (!existsSync(binary)) {
  die(
    `no standalone binary at ${binary} — build it first (cargo build -p freally-midi-master-plugin --release)`,
  );
}

// `dummy` rather than `auto`: a CI container has no sound card and no JACK
// server, and this gate is about the editor rather than the audio path. Naming
// the backend keeps a missing device from looking like a missing window.
const app = spawn(binary, ['--backend', 'dummy'], {
  stdio: ['ignore', 'inherit', 'inherit'],
  env: { ...process.env },
});

let exitedEarly = null;
app.on('exit', (code) => {
  exitedEarly = code;
});

try {
  console.log('waiting for the plugin window…');
  const started = Date.now();
  let id = null;
  while (!id && Date.now() - started < WINDOW_TIMEOUT_MS) {
    if (exitedEarly !== null)
      die(`the standalone exited before opening a window (code ${exitedEarly})`);
    await sleep(2_000);
    id = windowId();
  }
  if (!id)
    die(
      `no window titled "${WINDOW_TITLE}" appeared within ${WINDOW_TIMEOUT_MS / 60000} minutes`,
    );

  console.log(`window ${id} is up; letting it paint…`);
  await sleep(PAINT_MS);

  // Re-read the id: the webview is created on the GTK thread a moment after the
  // baseview window, so the id that exists first is not necessarily the one
  // worth photographing.
  const shot = sh('import', ['-window', windowId() ?? id, output]);
  if (shot.status !== 0)
    die(`window capture failed: ${shot.stderr || shot.stdout || 'no output'}`);
  if (!existsSync(output)) die('the capture tool reported success but wrote no file');

  const { size } = statSync(output);
  const png = readFileSync(output);
  const width = png.readUInt32BE(16);
  const height = png.readUInt32BE(20);

  const counted = sh('identify', ['-format', '%k', output]);
  const colours = Number.parseInt((counted.stdout ?? '').trim(), 10);

  console.log(
    `captured ${width}x${height}, ${Math.round(size / 1024)} KB, ${colours} distinct colours`,
  );

  if (width < 800 || height < 500) {
    die(`the capture is ${width}x${height} — too small to be the plugin window`);
  }
  if (!Number.isFinite(colours)) {
    die(`could not count colours: ${counted.stderr || 'no output from identify'}`);
  }
  // ⛔ The assertion this whole file exists for. Everything above passes with an
  // empty window.
  if (colours < MIN_COLOURS) {
    die(
      `the window rendered ${colours} distinct colours (needs ${MIN_COLOURS}) — ` +
        'it opened but drew nothing, which is the Linux editor failing, not passing',
    );
  }
} finally {
  app.kill();
}
