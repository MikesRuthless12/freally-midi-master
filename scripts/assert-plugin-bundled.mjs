#!/usr/bin/env node
/**
 * Refuse a plugin binary that has no UI or no dataset in it.
 *
 * The plugin equivalent of `assert-bundled.mjs`, and it exists for the same
 * reason that one did: the failure it catches looks exactly like a crash.
 *
 * `plugin/src/editor.rs` compiles `dist/` into the binary and
 * `plugin/src/dataset.rs` compiles `data/` into it, both with `include_dir!`.
 * Build the crate without running `npm run build` first and you get a plugin
 * that loads fine, opens a window, and renders nothing — no error in the DAW,
 * nothing in a console you cannot open. The dataset failing the same way gives
 * an empty roster with no explanation.
 *
 * Both are compile-time embeds, so both are checkable by looking at the bytes.
 * A second here beats ten minutes of diagnosing a blank window.
 *
 * Usage:
 *   node scripts/assert-plugin-bundled.mjs [path-to-binary]
 */

import { readFileSync, statSync } from 'node:fs';
import { argv, exit } from 'node:process';

/** Where cargo puts it, per platform, when no path is given. */
const DEFAULTS = [
  'target/release/freally_midi_master_plugin.dll',
  'target/release/libfreally_midi_master_plugin.so',
  'target/release/libfreally_midi_master_plugin.dylib',
];

/**
 * Markers that must appear in the binary, and what their absence means.
 *
 * Each is a string that only exists if the thing it stands for was embedded.
 * Deliberately not asset *filenames*: Vite hashes those, so a check against
 * one would have to be regenerated on every build and would rot into a
 * permanent skip.
 */
const REQUIRED = [
  {
    marker: 'clap_entry',
    what: 'the CLAP entry point',
    fix: 'the crate did not build as a cdylib — check `crate-type` in plugin/Cargo.toml',
  },
  {
    marker: 'GetPluginFactory',
    what: 'the VST3 entry point',
    fix:
      '`clap_wrapper::export_vst3!()` is missing from plugin/src/lib.rs. Without it ' +
      'Ableton, Logic, Pro Tools and Cubase can load nothing at all — none of them speaks CLAP',
  },
  {
    marker: 'com.mikeweaver.freally-midi-master',
    what: 'the plugin id',
    fix: 'ClapPlugin::CLAP_ID is missing, and no host will be able to identify this plugin',
  },
  {
    marker: '<!doctype html',
    what: 'the bundled UI (dist/index.html)',
    fix: 'run `npm run build` before `cargo build` — the plugin will open a blank window',
    caseInsensitive: true,
  },
  {
    marker: 'progressionFamilies',
    what: 'the bundled dataset (data/)',
    fix: 'data/ did not embed — the plugin will show an empty roster',
  },
];

function locate() {
  const given = argv[2];
  if (given) return given;

  for (const candidate of DEFAULTS) {
    try {
      if (statSync(candidate).isFile()) return candidate;
    } catch {
      // Not this platform's artefact. Keep looking.
    }
  }
  return null;
}

const path = locate();
if (!path) {
  console.error(
    'assert-plugin-bundled: no plugin binary found. Build one first:\n' +
      '  cargo build -p freally-midi-master-plugin --release',
  );
  exit(1);
}

let bytes;
try {
  bytes = readFileSync(path);
} catch (error) {
  console.error(`assert-plugin-bundled: could not read ${path}: ${error.message}`);
  exit(1);
}

// latin1 rather than utf8: the binary is not text, and utf8 decoding mangles
// bytes that happen to be invalid sequences — including, sometimes, the ones
// inside the string being searched for.
const text = bytes.toString('latin1');

const missing = REQUIRED.filter(({ marker, caseInsensitive }) =>
  caseInsensitive ? !text.toLowerCase().includes(marker) : !text.includes(marker),
);

if (missing.length > 0) {
  console.error(`assert-plugin-bundled: ${path} is not shippable.\n`);
  for (const { what, fix } of missing) {
    console.error(`  ✗ ${what} is missing`);
    console.error(`    ${fix}\n`);
  }
  exit(1);
}

const mb = (bytes.length / 1024 / 1024).toFixed(1);
console.log(`assert-plugin-bundled: ${path} looks shippable (${mb} MB)`);
for (const { what } of REQUIRED) console.log(`  ✓ ${what}`);
