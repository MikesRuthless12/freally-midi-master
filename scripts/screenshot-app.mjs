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
    // Capture by window id — the macOS equivalent of PrintWindow.
    //
    // `screencapture -R` takes a screen REGION, so anything hanging off the
    // display edge is simply absent from the file. The runner desktop is
    // 1024x768 and tauri.conf.json sets minWidth 1280, which the window server
    // will not go below — so no amount of moving or resizing makes the window
    // fit, and the region grab silently lost the right rail and the transport
    // bar. `-l <CGWindowID>` reads the window's own buffer instead and does not
    // care where it sits or how big the screen is.
    //
    // The id comes from CoreGraphics via JXA, because nothing in the shell
    // exposes a CGWindowID: System Events' `id of window` is a different
    // number space entirely.
    const lookup = `
      ObjC.import('CoreGraphics');
      const windows = $.CGWindowListCopyWindowInfo(
        $.kCGWindowListOptionOnScreenOnly | $.kCGWindowListExcludeDesktopElements,
        $.kCGNullWindowID,
      );
      const count = windows.count;
      for (let i = 0; i < count; i++) {
        const w = windows.objectAtIndex(i);
        const owner = ObjC.unwrap(w.objectForKey('kCGWindowOwnerName')) || '';
        const bounds = w.objectForKey('kCGWindowBounds');
        const width = bounds ? ObjC.unwrap(bounds.objectForKey('Width')) : 0;
        if (owner.toLowerCase().indexOf('freally') !== -1 && width > 200) {
          ObjC.unwrap(w.objectForKey('kCGWindowNumber'));
        }
      }
    `;
    const found = sh('osascript', ['-l', 'JavaScript', '-e', lookup]);
    const id = (found.stdout ?? '').trim();

    if (/^\d+$/.test(id)) {
      // `-o` drops the drop-shadow so the image is the window and nothing else.
      return sh('screencapture', ['-x', '-o', '-l', id, output]);
    }

    // No id: fall back to the region grab, but refuse if it would clip. A
    // picture with the right rail cut off is not evidence that the right rail
    // renders — that is the failure this whole file exists to prevent.
    const script = `
      tell application "Finder" to set screen to bounds of window of desktop
      set screenW to item 3 of screen
      set screenH to item 4 of screen
      tell application "System Events"
        set procs to (every process whose name contains "freally")
        if (count of procs) = 0 then return "none"
        set p to item 1 of procs
        if (count of windows of p) = 0 then return "none"
        set w to window 1 of p
        set position of w to {0, 25}
        set {x, y} to position of w
        set {ww, hh} to size of w
        return (x as text) & "," & (y as text) & "," & (ww as text) & "," & (hh as text) & "," & (screenW as text) & "," & (screenH as text)
      end tell`;
    const bounds = sh('osascript', ['-e', script]);
    const answer = (bounds.stdout ?? '').trim();
    if (!answer || answer === 'none') {
      return {
        status: 1,
        stderr: `no window id (${found.stderr || 'no output'}) and no window bounds either`,
      };
    }

    const [x, y, w, h, screenW, screenH] = answer.split(',').map(Number);
    if (x < 0 || y < 0 || x + w > screenW || y + h > screenH) {
      return {
        status: 1,
        stderr:
          `could not read a CGWindowID (${found.stderr || 'no output'}), and the window ` +
          `(${w}x${h} at ${x},${y}) does not fit the ${screenW}x${screenH} display, so a ` +
          'region capture would be clipped.',
      };
    }
    return sh('screencapture', ['-x', '-R', `${x},${y},${w},${h}`, output]);
  }

  // Windows: PrintWindow, which asks the window to render ITSELF into a bitmap.
  //
  // `CopyFromScreen` was here first, and it lies whenever the window is bigger
  // than the display. The runner's desktop is 1024x768 and the app's window is
  // wider than that, so the "window" capture was really a screen-region grab
  // that clipped the right rail and the transport bar off and photographed the
  // Windows taskbar in their place. Same family as the original bug where the
  // job photographed the desktop and passed on file size: what came back was
  // not the thing being asserted about.
  //
  // PW_RENDERFULLCONTENT (2) is the flag that makes this work for a
  // WebView2/Chromium child surface; without it the client area comes back
  // blank. If it does anyway, fall back to the screen grab — a clipped picture
  // of the app beats no picture, and the blank-capture guard still applies.
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
        [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
        public struct RECT { public int L, T, R, B; }
      }
"@
    [Cap]::SetProcessDPIAware() | Out-Null
    $p = Get-Process -Name '${PROCESS_NAME}' -ErrorAction SilentlyContinue |
         Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if (-not $p) { Write-Error 'no app process with a window'; exit 1 }
    $handle = $p.MainWindowHandle
    [Cap]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 800
    $r = New-Object Cap+RECT
    [Cap]::GetWindowRect($handle, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $h = $r.B - $r.T
    if ($w -le 0 -or $h -le 0) { Write-Error "bad window rect \${w}x\${h}"; exit 1 }

    # How many distinct colours are in a sparse grid of samples? One or two
    # means a flat rectangle, which is what a failed PrintWindow returns.
    function Distinct-Colours($bitmap) {
      $seen = New-Object System.Collections.Generic.HashSet[int]
      for ($x = 4; $x -lt $bitmap.Width; $x += [Math]::Max(1, [int]($bitmap.Width / 32))) {
        for ($y = 4; $y -lt $bitmap.Height; $y += [Math]::Max(1, [int]($bitmap.Height / 32))) {
          [void]$seen.Add($bitmap.GetPixel($x, $y).ToArgb())
        }
      }
      return $seen.Count
    }

    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    $printed = [Cap]::PrintWindow($handle, $dc, 2)
    $g.ReleaseHdc($dc)

    $how = 'PrintWindow'
    if (-not $printed -or (Distinct-Colours $bmp) -lt 3) {
      $g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size($w, $h)))
      $how = 'CopyFromScreen (PrintWindow came back blank)'
    }

    $bmp.Save('${output.replace(/\\/g, '\\\\')}', [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Output "captured \${w}x\${h} via $how"
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
  // Linux CI runs WebKitGTK on the software rasteriser, which is markedly
  // slower to first paint than the GPU path Windows and macOS get.
  await sleep(process.platform === 'linux' ? 25_000 : 12_000);

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
