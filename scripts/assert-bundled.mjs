#!/usr/bin/env node
/**
 * Refuse to run a binary that has no UI inside it.
 *
 * There are two ways to produce `target/release/freally-midi-master.exe` and
 * only one of them works:
 *
 *   npm run tauri build   → runs the frontend build and EMBEDS dist/ in the exe
 *   cargo build --release → compiles the Rust only; the exe still points at the
 *                           Vite dev server on localhost:1420
 *
 * Launched on its own, the second shows WebView2's "Hmmm… can't reach this
 * page" with a blue Refresh button. It looks exactly like the app crashing on
 * startup, and it has now cost two separate debugging sessions — once with the
 * debug exe, once with a release exe built the wrong way, which is worse
 * because "release" implies it is the shippable one.
 *
 * The check: a bundled exe contains the hashed asset filenames from dist/. A
 * cargo-only build does not. That is cheap, needs no launch, and cannot be
 * fooled by a stale binary of the same size.
 *
 * Usage: node scripts/assert-bundled.mjs [path-to-exe]
 */

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const exe =
  process.argv[2] ??
  join(
    root,
    'target',
    'release',
    process.platform === 'win32' ? 'freally-midi-master.exe' : 'freally-midi-master',
  );

function fail(message) {
  console.error(`error: ${message}`);
  console.error('\nBuild it properly first:\n  npm run tauri build\n');
  process.exit(1);
}

if (!existsSync(exe)) fail(`no binary at ${exe}`);

const assets = join(root, 'dist', 'assets');
if (!existsSync(assets)) fail('dist/assets is missing — the frontend has never been built');

// The main JS chunk, whose name carries a content hash.
const marker = readdirSync(assets).find((f) => /^index-.*\.js$/.test(f));
if (!marker) fail('no hashed index-*.js in dist/assets — is the frontend build broken?');

// Search the binary as bytes; the asset table is not UTF-8 text.
const haystack = readFileSync(exe);
if (!haystack.includes(Buffer.from(marker, 'utf8'))) {
  fail(
    `${exe}\n       does not contain the frontend (looked for "${marker}").\n` +
      "       This is a cargo-only build: launching it shows WebView2's\n" +
      '       "can\'t reach this page" because it points at the dev server.',
  );
}

console.log(`ok: ${exe} has the frontend embedded (${marker})`);
