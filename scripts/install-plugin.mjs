#!/usr/bin/env node
/**
 * Put the built plugin where the DAWs look, on every build.
 *
 * The problem this solves: copying a 23 MB binary into a plugin folder and
 * rescanning after every build is slow enough that people stop testing. This
 * links (or copies) it once per build into every configured destination, so a
 * rebuild is immediately what the DAW loads.
 *
 * A **symlink** is preferred, because then the DAW is always pointed at the
 * newest build and this script's work is done after the first run. A **copy**
 * is the fallback where linking is refused — notably `C:\Program Files\...`,
 * which needs an elevated shell.
 *
 * Destinations come from `plugin-install.json` at the repo root if it exists,
 * so machine-specific paths are not baked into a script everyone shares. See
 * `plugin-install.example.json`.
 *
 * Usage:
 *   node scripts/install-plugin.mjs                  # release, configured dirs
 *   node scripts/install-plugin.mjs --debug          # the debug build
 *   node scripts/install-plugin.mjs --dir "D:\\VST"  # one extra destination
 *   node scripts/install-plugin.mjs --copy           # copy instead of link
 *   node scripts/install-plugin.mjs --remove         # take them back out
 */

import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
} from 'node:fs';
import { homedir, platform } from 'node:os';
import { join } from 'node:path';
import { argv, cwd, exit } from 'node:process';

const args = argv.slice(2);
const has = (flag) => args.includes(flag);
const profile = has('--debug') ? 'debug' : 'release';

/**
 * What gets installed, in the order a DAW is likely to want it.
 *
 * **The VST3 does not exist yet**, and its absence here is expected rather than
 * a failure: nih-plug's VST3 export links `vst3-sys`, which is GPLv3 and
 * incompatible with this project's licence, so VST3 and AU are projected from
 * the CLAP by `clap-wrapper` instead (TASK-P08). The moment that lands and
 * writes a `.vst3` into `target/bundled/`, this script picks it up with no
 * change — which is why it is listed now and merely reported as missing.
 */
const ARTIFACTS = [
  { file: 'Freally MIDI Master.clap', label: 'CLAP' },
  { file: 'Freally MIDI Master.vst3', label: 'VST3', pending: 'TASK-P08 (clap-wrapper)' },
];

/** Where cargo leaves the shared library this platform loads. */
function builtLibrary() {
  const names = {
    win32: 'freally_midi_master_plugin.dll',
    darwin: 'libfreally_midi_master_plugin.dylib',
    linux: 'libfreally_midi_master_plugin.so',
  };
  const name = names[platform()];
  if (!name) {
    console.error(`install-plugin: unsupported platform ${platform()}`);
    exit(1);
  }
  return join(cwd(), 'target', profile, name);
}

/**
 * The **per-user** directory each OS specifies for CLAP plugins.
 *
 * Deliberately the user-level path rather than the system one. The CLAP spec
 * defines both and CLAP hosts scan both — but the system path lives under
 * `C:\Program Files` / `/Library`, which needs an elevated shell to write. A
 * default that fails with EPERM on a normal terminal is a default that does
 * not install anything, which defeats running this on every build.
 *
 * Add the system path to `plugin-install.json` if you want it as well; it will
 * be attempted, and reported rather than fatal when it is refused.
 *
 * **This folder only helps a host that speaks CLAP.** Bitwig, Reaper,
 * FL Studio 21.2+, Studio One 6.5+ and Renoise do. **Ableton Live, Logic,
 * Pro Tools and Cubase do not** — pointing Live's "VST3 custom folder" here
 * cannot work, because Live is looking for a `.vst3` and a `.clap` is a
 * different format wearing a different extension. Those hosts need the VST3
 * that `clap-wrapper` produces (TASK-P08), and until it exists there is
 * nothing this script can put anywhere that they will load.
 */
function defaultClapDir() {
  switch (platform()) {
    case 'win32':
      return join(
        process.env.LOCALAPPDATA ?? join(homedir(), 'AppData', 'Local'),
        'Programs',
        'Common',
        'CLAP',
      );
    case 'darwin':
      return join(homedir(), 'Library', 'Audio', 'Plug-Ins', 'CLAP');
    default:
      return join(homedir(), '.clap');
  }
}

/**
 * Every destination to install into.
 *
 * The OS-standard directory always, plus whatever `plugin-install.json` adds.
 * A bad config must not stop the build, so it is reported and skipped.
 */
function destinations() {
  const dirs = [defaultClapDir()];

  const configPath = join(cwd(), 'plugin-install.json');
  if (existsSync(configPath)) {
    try {
      const config = JSON.parse(readFileSync(configPath, 'utf8'));
      for (const dir of config.directories ?? []) dirs.push(dir);
    } catch (error) {
      console.warn(`install-plugin: ignoring plugin-install.json (${error.message})`);
    }
  }

  const flag = args.indexOf('--dir');
  if (flag !== -1 && args[flag + 1]) dirs.push(args[flag + 1]);

  return [...new Set(dirs)];
}

function isLink(path) {
  try {
    return lstatSync(path).isSymbolicLink();
  } catch {
    return false;
  }
}

/** Place one artifact in one directory. Returns a short status for the log. */
function place(source, dir, name) {
  const target = join(dir, name);

  // Always clear what is there first. A stale *copy* sitting where a link
  // should be is the version of this that wastes an afternoon: the DAW keeps
  // loading last week's build and every change appears to do nothing.
  if (existsSync(target) || isLink(target)) {
    try {
      rmSync(target, { force: true, recursive: true });
    } catch (error) {
      return { ok: false, why: `could not replace: ${error.code ?? error.message}` };
    }
  }

  if (has('--copy')) {
    try {
      copyFileSync(source, target);
      return { ok: true, how: 'copied' };
    } catch (error) {
      return { ok: false, why: `${error.code ?? error.message}` };
    }
  }

  try {
    symlinkSync(source, target, 'file');
    return { ok: true, how: 'linked' };
  } catch (error) {
    // Linking into `C:\Program Files\...` needs an elevated shell. Falling
    // back to a copy is better than failing the build — it is stale-able, but
    // this script runs on every build, so it is refreshed every build.
    try {
      copyFileSync(source, target);
      return { ok: true, how: 'copied (link refused)' };
    } catch (copyError) {
      return {
        ok: false,
        why: `${error.code ?? error.message} / ${copyError.code ?? copyError.message}`,
      };
    }
  }
}

const dirs = destinations();

if (has('--remove')) {
  for (const dir of dirs) {
    for (const { file } of ARTIFACTS) {
      const target = join(dir, file);
      if (existsSync(target) || isLink(target)) {
        rmSync(target, { force: true, recursive: true });
        console.log(`install-plugin: removed ${target}`);
      }
    }
  }
  exit(0);
}

const library = builtLibrary();
if (!existsSync(library)) {
  console.error(
    `install-plugin: ${library} does not exist. Build it first:\n  npm run plugin:build`,
  );
  exit(1);
}

// On Windows and Linux a `.clap` is the shared library under another name; on
// macOS it is a bundle, which `clap-wrapper` produces (TASK-P08).
const bundled = join(cwd(), 'target', 'bundled');
mkdirSync(bundled, { recursive: true });
const clapPath = join(bundled, ARTIFACTS[0].file);
if (platform() !== 'darwin') copyFileSync(library, clapPath);

let failures = 0;

for (const { file, label, pending } of ARTIFACTS) {
  const source = join(bundled, file);
  if (!existsSync(source)) {
    // Expected while a format is not built yet. Said out loud rather than
    // skipped silently, so "my DAW cannot see the VST3" has an answer here.
    console.log(
      `install-plugin: no ${label} yet${pending ? ` — arrives with ${pending}` : ''}`,
    );
    continue;
  }

  for (const dir of dirs) {
    try {
      mkdirSync(dir, { recursive: true });
    } catch (error) {
      console.error(`install-plugin: ✗ ${dir} — ${error.code ?? error.message}`);
      failures += 1;
      continue;
    }

    const result = place(source, dir, file);
    if (result.ok) {
      console.log(`install-plugin: ✓ ${label} ${result.how} -> ${join(dir, file)}`);
    } else {
      console.error(`install-plugin: ✗ ${label} -> ${join(dir, file)} — ${result.why}`);
      if (platform() === 'win32' && dir.toLowerCase().includes('program files')) {
        console.error(
          '    Writing under Program Files needs an elevated shell. Either run this\n' +
            '    terminal as Administrator once, or point plugin-install.json at a\n' +
            "    folder you own and add it to your DAW's plugin search paths.",
        );
      }
      failures += 1;
    }
  }
}

// Never fail the build over an install destination: the binary is built and
// `npm run plugin:standalone` still runs it. The message above is the point.
if (failures > 0) console.error(`install-plugin: ${failures} destination(s) failed`);
