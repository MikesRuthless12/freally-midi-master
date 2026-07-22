#!/usr/bin/env node
/**
 * Launch the real app and photograph **its window**.
 *
 * CI runs this on all three OSes so a human can look at three PNGs and see the
 * Studio actually rendering, rather than inferring it from a green tick. It
 * catches the class of failure every other gate misses: the code compiles, the
 * DOM tests pass against a browser, and the native window comes up blank.
 *
 * Two things this gets right, both learned the hard way:
 *
 * 1. **Wait for the window, do not sleep and do not watch the process.** The
 *    first version slept 45s after Vite was up; on a cold Linux runner cargo was
 *    still on crate 306 of 576. The second polled for the process — but
 *    `pgrep -f` matches the whole command line, so it matched cargo *building* a
 *    crate of that name and fired while compilation was a fifth done. The window
 *    is the actual precondition, so that is what it waits for.
 *
 * 2. **Capture the window, not the screen.** The first version grabbed the
 *    whole desktop, so Windows and macOS "passed" with a picture of the
 *    runner's log console and no app in it at all. A screenshot job that
 *    photographs the wrong thing is worse than none: it manufactures
 *    confidence. Capturing by window handle means a pass cannot be faked by a
 *    desktop wallpaper.
 *
 * Usage: node scripts/screenshot-app.mjs <output.png>
 */

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(process.argv[2] ?? 'app-screenshot.png');
mkdirSync(dirname(output), { recursive: true });

/** The window title set in tauri.conf.json. */
const WINDOW_TITLE = 'Freally MIDI Master';
const PROCESS_NAME = 'freally-midi-master';

/** Cargo can be building for a long time on a cold runner. */
const APP_TIMEOUT_MS = 15 * 60 * 1000;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function die(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

function sh(cmd, args) {
  return spawnSync(cmd, args, { encoding: 'utf8' });
}

/**
 * Has the app opened a window yet?
 *
 * Waiting on the *window* rather than the process, because the process is not
 * the precondition — the window is, and there is a long gap between the two
 * while the WebView starts.
 *
 * It also avoids a trap: `pgrep -f` matches the whole command line, so it
 * matched `cargo` building a crate called freally-midi-master and reported the
 * app as up while compilation was still on crate 119 of 576. Windows was
 * unaffected because tasklist matches the image name exactly, which is why that
 * leg passed and the other two did not.
 */
function appWindowExists() {
  if (process.platform === 'linux') {
    const find = sh('xdotool', ['search', '--name', WINDOW_TITLE]);
    return find.status === 0 && (find.stdout ?? '').trim().length > 0;
  }

  if (process.platform === 'darwin') {
    const probe = sh('osascript', [
      '-e',
      `tell application "System Events" to return (count of (every process whose name contains "${PROCESS_NAME}"))`,
    ]);
    return Number.parseInt((probe.stdout ?? '0').trim(), 10) > 0;
  }

  const ps = sh('powershell', [
    '-NoProfile',
    '-Command',
    `@(Get-Process -Name '${PROCESS_NAME}' -ErrorAction SilentlyContinue |` +
      ` Where-Object { $_.MainWindowHandle -ne 0 }).Count`,
  ]);
  return Number.parseInt((ps.stdout ?? '0').trim(), 10) > 0;
}

async function waitFor(label, predicate, timeoutMs) {
  const started = Date.now();
  let lastLog = 0;
  while (Date.now() - started < timeoutMs) {
    if (predicate()) return true;
    const elapsed = Math.round((Date.now() - started) / 1000);
    if (elapsed - lastLog >= 30) {
      console.log(`  still waiting for ${label}… (${elapsed}s)`);
      lastLog = elapsed;
    }
    await sleep(3_000);
  }
  return false;
}

/** Capture only the app's window. Returns a spawnSync-ish result. */
function captureWindow() {
  if (process.platform === 'linux') {
    // xdotool finds the window by its title; import grabs that window alone.
    const find = sh('xdotool', ['search', '--name', WINDOW_TITLE]);
    const id = (find.stdout ?? '').trim().split('\n').filter(Boolean).pop();
    if (!id) return { status: 1, stderr: `xdotool found no window titled "${WINDOW_TITLE}"` };
    return sh('import', ['-window', id, output]);
  }

  if (process.platform === 'darwin') {
    // Ask the window server for the app's window bounds, then grab that rect.
    const script = `
      tell application "System Events"
        set procs to (every process whose name contains "freally")
        if (count of procs) = 0 then return "none"
        set p to item 1 of procs
        if (count of windows of p) = 0 then return "none"
        set w to window 1 of p
        set {x, y} to position of w
        set {ww, hh} to size of w
        return (x as text) & "," & (y as text) & "," & (ww as text) & "," & (hh as text)
      end tell`;
    const bounds = sh('osascript', ['-e', script]);
    const rect = (bounds.stdout ?? '').trim();
    if (!rect || rect === 'none') {
      return { status: 1, stderr: `could not find the app window: ${bounds.stderr || rect}` };
    }
    return sh('screencapture', ['-x', '-R', rect, output]);
  }

  // Windows: PrintWindow into a bitmap sized to the window itself.
  const ps = `
    $ErrorActionPreference = 'Stop'
    Add-Type -AssemblyName System.Drawing
    Add-Type @"
      using System;
      using System.Runtime.InteropServices;
      public class Cap {
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
        public struct RECT { public int L, T, R, B; }
      }
"@
    [Cap]::SetProcessDPIAware() | Out-Null
    $p = Get-Process -Name '${PROCESS_NAME}' -ErrorAction SilentlyContinue |
         Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if (-not $p) { Write-Error 'no app process with a window'; exit 1 }
    [Cap]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 800
    $r = New-Object Cap+RECT
    [Cap]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $h = $r.B - $r.T
    if ($w -le 0 -or $h -le 0) { Write-Error "bad window rect \${w}x\${h}"; exit 1 }
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $bmp.Save('${output.replace(/\\/g, '\\\\')}', [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Output "captured \${w}x\${h}"
  `;
  return sh('powershell', ['-NoProfile', '-Command', ps]);
}

/** PNG dimensions, straight out of the IHDR chunk. */
function pngSize(path) {
  const b = readFileSync(path);
  return { width: b.readUInt32BE(16), height: b.readUInt32BE(20) };
}

const app = spawn('npm', ['run', 'tauri', 'dev'], {
  cwd: root,
  stdio: ['ignore', 'inherit', 'inherit'],
  shell: process.platform === 'win32',
  env: { ...process.env, CI: 'true' },
});

let exitedEarly = null;
app.on('exit', (code) => {
  exitedEarly = code;
});

try {
  console.log('waiting for the app window (cargo may still be building)…');
  const running = await waitFor('the app window', appWindowExists, APP_TIMEOUT_MS);

  if (exitedEarly !== null) {
    die(`\`tauri dev\` exited before the app started (code ${exitedEarly})`);
  }
  if (!running) {
    die(`no app window appeared within ${APP_TIMEOUT_MS / 60000} minutes`);
  }

  console.log('app is up; letting the window paint…');
  await sleep(12_000);

  const shot = captureWindow();
  if (shot.status !== 0) {
    die(`window capture failed: ${shot.stderr || shot.stdout || 'no output'}`);
  }
  if (!existsSync(output)) {
    die('the capture tool reported success but wrote no file');
  }

  const { size } = statSync(output);
  const { width, height } = pngSize(output);
  console.log(`captured ${width}x${height}, ${Math.round(size / 1024)} KB`);

  // The window is at least 1280x760 by config. Anything much smaller is not
  // the app, and a near-empty file is a blank window.
  if (width < 800 || height < 500) {
    die(`the capture is ${width}x${height} — too small to be the app window`);
  }
  if (size < 10_000) {
    die(`the capture is only ${size} bytes — the window is probably blank`);
  }
} finally {
  app.kill();
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/F', '/IM', `${PROCESS_NAME}.exe`], { stdio: 'ignore' });
  } else {
    spawnSync('pkill', ['-f', PROCESS_NAME], { stdio: 'ignore' });
  }
}
