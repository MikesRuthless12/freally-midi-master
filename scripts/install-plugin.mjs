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
  cpSync,
  statSync,
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
 * What gets installed. One built library, two formats.
 *
 * `nih_export_clap!` and `clap_wrapper::export_vst3!` both live in the same
 * cdylib, so the `.clap` and the `.vst3` are that one file under two names —
 * nothing is compiled twice. VST3 comes from `clap-wrapper` (MIT/Apache-2.0
 * over Steinberg's MIT SDK) rather than from nih-plug's own export, which
 * links the GPLv3 `vst3-sys` and is disabled for that reason.
 *
 * Each format installs into its **own** directory: a VST3 in a CLAP folder is
 * invisible to every host, and the reverse is what made Ableton show nothing.
 */
const ARTIFACTS = [
  { file: 'Freally MIDI Master.clap', label: 'CLAP', kind: 'CLAP' },
  { file: 'Freally MIDI Master.vst3', label: 'VST3', kind: 'VST3', directory: true },
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
 * **The CLAP folder only helps a host that speaks CLAP.** Bitwig, Reaper,
 * FL Studio 21.2+, Studio One 6.5+ and Renoise do. **Ableton Live, Logic,
 * Pro Tools and Cubase do not** — pointing Live's "VST3 custom folder" at the
 * CLAP directory cannot work, because Live is looking for a `.vst3` and a
 * `.clap` is a different format wearing a different extension. Those hosts
 * load the VST3 bundle from the VST3 directory below.
 */
function defaultDir(kind) {
  switch (platform()) {
    case 'win32':
      return join(
        process.env.LOCALAPPDATA ?? join(homedir(), 'AppData', 'Local'),
        'Programs',
        'Common',
        kind,
      );
    case 'darwin':
      return join(homedir(), 'Library', 'Audio', 'Plug-Ins', kind);
    default:
      return kind === 'VST3' ? join(homedir(), '.vst3') : join(homedir(), '.clap');
  }
}

/**
 * Every destination to install into.
 *
 * The OS-standard directory always, plus whatever `plugin-install.json` adds.
 * A bad config must not stop the build, so it is reported and skipped.
 */
function destinations(kind) {
  const dirs = [defaultDir(kind)];

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

  // A VST3 is a bundle *directory*; a CLAP on Windows and Linux is a file.
  const isDir = statSync(source).isDirectory();
  const put = () =>
    isDir ? cpSync(source, target, { recursive: true }) : copyFileSync(source, target);

  if (has('--copy')) {
    try {
      put();
      return { ok: true, how: 'copied' };
    } catch (error) {
      return { ok: false, why: `${error.code ?? error.message}` };
    }
  }

  try {
    symlinkSync(source, target, isDir ? 'junction' : 'file');
    return { ok: true, how: 'linked' };
  } catch (error) {
    // Linking into `C:\Program Files\...` needs an elevated shell, and plain
    // symlinks need Developer Mode on Windows. Falling back to a copy is
    // better than failing the build — it can go stale, but this runs on every
    // build, so it is refreshed on every build.
    try {
      put();
      return { ok: true, how: 'copied (link refused)' };
    } catch (copyError) {
      return {
        ok: false,
        why: `${error.code ?? error.message} / ${copyError.code ?? copyError.message}`,
      };
    }
  }
}

if (has('--remove')) {
  for (const { file, kind } of ARTIFACTS) {
    for (const dir of destinations(kind)) {
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

// One binary, two formats: `nih_export_clap!` and `clap_wrapper::export_vst3!`
// both live in the same cdylib, so the `.clap` and the `.vst3` are that file
// under two names. Nothing is compiled twice.
const bundled = join(cwd(), 'target', 'bundled');
mkdirSync(bundled, { recursive: true });

// CLAP: a plain file on Windows and Linux.
if (platform() !== 'darwin') copyFileSync(library, join(bundled, ARTIFACTS[0].file));

// VST3: a **bundle directory**, not a plain file. The format has required this
// shape since VST3 3.6.10, and while some hosts still accept a bare `.vst3`
// DLL, Ableton Live is not reliably one of them — a loose file is the version
// of this that silently fails to appear in the browser.
//
//   Freally MIDI Master.vst3/
//     Contents/
//       x86_64-win/
//         Freally MIDI Master.vst3   <- the library
const VST3_ARCH = {
  win32: 'x86_64-win',
  linux: 'x86_64-linux',
  darwin: 'MacOS',
};
const vst3Root = join(bundled, ARTIFACTS[1].file);
const vst3Binary = join(vst3Root, 'Contents', VST3_ARCH[platform()] ?? 'x86_64-win');

// The bundle is *updated in place*, never removed and rebuilt. Once it has
// been installed as a junction, a DAW holding the plugin open locks the file —
// and `rmSync` on the tree then fails with EPERM, which stops the build over
// something that did not need doing. Overwriting the one file inside it is all
// this ever needed.
mkdirSync(vst3Binary, { recursive: true });
try {
  copyFileSync(library, join(vst3Binary, ARTIFACTS[1].file));
} catch (error) {
  console.error(
    `install-plugin: could not update the VST3 (${error.code ?? error.message}).\n` +
      '  A DAW almost certainly has it loaded. Close the host and build again —\n' +
      '  Windows will not let anyone overwrite a DLL that is mapped into a\n' +
      '  running process.',
  );
  exit(1);
}

let failures = 0;

for (const { file, label, kind } of ARTIFACTS) {
  const source = join(bundled, file);
  if (!existsSync(source)) {
    // Both formats come out of one build, so a missing one is a broken build
    // rather than a format that has not landed yet. Said out loud, because
    // "my DAW cannot see it" needs an answer here rather than in the DAW.
    console.error(`install-plugin: no ${label} was produced — run npm run plugin:build`);
    failures += 1;
    continue;
  }

  for (const dir of destinations(kind)) {
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
