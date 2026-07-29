#!/usr/bin/env node
/**
 * Launch the plugin's standalone build and photograph **its own window**, on
 * Windows, macOS and Linux.
 *
 * This is TASK-P12's gate grown into the whole matrix, and the reason it exists
 * is that a plugin which compiles proves nothing at all. The failures worth
 * catching are: GTK never started, the webview was never built, the page was
 * never fetched, WebView2 is missing, the custom protocol 404s. **Every one of
 * those compiles cleanly and every one of them is an empty window.**
 *
 * So the assertion is not "it built" and not even "a window exists" — it is
 * **the window has a picture in it**. A window that opened but never rendered is
 * one flat rectangle of the app's own background colour; the UI is thousands of
 * colours. That is the difference this gate has to tell apart, and it is the
 * only one visible from outside the process.
 *
 * ## Two things this gets right, both learned the hard way
 *
 * 1. **Capture the window, not the screen.** An earlier screenshot job grabbed
 *    the whole desktop, so Windows and macOS "passed" with a picture of the
 *    runner's log console and no app in it. A job that photographs the wrong
 *    thing is worse than none: it manufactures confidence.
 * 2. **Count the colours here, not with ImageMagick.** `identify` is not on the
 *    macOS runner and installing it costs minutes per job — but more
 *    importantly, three platforms asserting via three different tools is three
 *    chances for one of them to be measuring something else. The PNG is decoded
 *    below in about sixty lines of `zlib`, so the *same* number is computed the
 *    same way everywhere.
 *
 * Usage: node scripts/screenshot-plugin.mjs <output.png> [path/to/standalone]
 */

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { inflateSync } from 'node:zlib';

const output = resolve(process.argv[2] ?? 'screenshots/plugin.png');
const isWindows = process.platform === 'win32';
const fallbackBinary = `target/release/standalone${isWindows ? '.exe' : ''}`;
const binary = resolve(process.argv[3] ?? process.env.FREALLY_STANDALONE ?? fallbackBinary);
mkdirSync(dirname(output), { recursive: true });

/** `Plugin::NAME`, which nih-plug's standalone uses as its window title. */
const WINDOW_TITLE = 'Freally MIDI Master';

/** The binary's own name, which is what the process tables match on. */
const PROCESS_NAME = 'standalone';

const WINDOW_TIMEOUT_MS = 3 * 60 * 1000;

/** WebKitGTK on a software rasteriser is markedly slower to first paint. */
const PAINT_MS = process.platform === 'linux' ? 25_000 : 12_000;

/**
 * How many distinct colours mean "rendered".
 *
 * A blank window is 1 — the background this plugin sets deliberately, so that a
 * slow first paint is the app's colour rather than a white flash. Anti-aliasing
 * and a window border can lift that to a handful, so the bar sits well above the
 * noise and far below what any real frame of the UI produces.
 */
const MIN_COLOURS = 24;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function die(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

function sh(cmd, args) {
  const done = spawnSync(cmd, args, { encoding: 'utf8' });
  // ⛔ Name the missing tool. `spawnSync` reports ENOENT as a null status with
  // empty streams, so the caller's `stderr || stdout || 'no output'` printed
  // exactly `window capture failed: no output` — which cost a full CI round trip
  // to work out meant "`import` is not installed".
  if (done.error) {
    return {
      ...done,
      status: done.status ?? 1,
      stderr: `could not run \`${cmd}\`: ${done.error.message}`,
    };
  }
  return done;
}

/**
 * The number of distinct colours in a sparse grid of samples.
 *
 * A hand-rolled PNG reader, so the assertion is identical on all three
 * platforms and needs nothing installed. Returns `null` for a PNG shape it
 * cannot read, which the caller treats as a failure rather than a pass — an
 * unreadable capture is not evidence of a rendered window.
 */
function distinctColours(png) {
  if (png.length < 33 || png.readUInt32BE(0) !== 0x89504e47) return null;

  const width = png.readUInt32BE(16);
  const height = png.readUInt32BE(20);
  const depth = png[24];
  const colourType = png[25];
  const interlaced = png[28];

  // 8-bit RGB or RGBA, non-interlaced, which is what every capture tool here
  // writes. Anything else is refused rather than guessed at.
  if (depth !== 8 || (colourType !== 2 && colourType !== 6) || interlaced !== 0) return null;
  const channels = colourType === 6 ? 4 : 3;

  const parts = [];
  let at = 8;
  while (at + 8 <= png.length) {
    const length = png.readUInt32BE(at);
    const type = png.toString('ascii', at + 4, at + 8);
    if (type === 'IDAT') parts.push(png.subarray(at + 8, at + 8 + length));
    if (type === 'IEND') break;
    at += 12 + length;
  }
  if (parts.length === 0) return null;

  let raw;
  try {
    raw = inflateSync(Buffer.concat(parts));
  } catch {
    return null;
  }

  const stride = width * channels;
  if (raw.length < height * (stride + 1)) return null;

  // Un-filter, which PNG requires before any pixel means anything.
  const pixels = Buffer.alloc(height * stride);
  let read = 0;
  for (let y = 0; y < height; y += 1) {
    const filter = raw[read];
    read += 1;
    const line = raw.subarray(read, read + stride);
    read += stride;

    const row = pixels.subarray(y * stride, (y + 1) * stride);
    const above = y > 0 ? pixels.subarray((y - 1) * stride, y * stride) : null;

    for (let i = 0; i < stride; i += 1) {
      const left = i >= channels ? row[i - channels] : 0;
      const up = above ? above[i] : 0;
      const upLeft = above && i >= channels ? above[i - channels] : 0;
      let value = line[i];

      if (filter === 1) value += left;
      else if (filter === 2) value += up;
      else if (filter === 3) value += (left + up) >> 1;
      else if (filter === 4) {
        const p = left + up - upLeft;
        const dl = Math.abs(p - left);
        const du = Math.abs(p - up);
        const dul = Math.abs(p - upLeft);
        value += dl <= du && dl <= dul ? left : du <= dul ? up : upLeft;
      } else if (filter !== 0) return null;

      row[i] = value & 0xff;
    }
  }

  // ⛔ Sample the window's **interior**, inset from every edge.
  //
  // The Windows path can fall back to `CopyFromScreen`, which copies the screen
  // rectangle the window occupies — and at the edges that includes whatever is
  // behind and around it. A blank black window over a colourful terminal
  // measured 33–64 "distinct colours" and sailed past this gate, which is the
  // same class of failure as photographing the desktop and passing on file size.
  // Insetting by a tenth throws away the frame, the title bar and any bleed.
  const inset = 0.1;
  const left = Math.floor(width * inset);
  const right = Math.ceil(width * (1 - inset));
  const top = Math.floor(height * inset);
  const bottom = Math.ceil(height * (1 - inset));

  const seen = new Set();
  const stepX = Math.max(1, Math.floor((right - left) / 200));
  const stepY = Math.max(1, Math.floor((bottom - top) / 200));
  for (let y = top; y < bottom; y += stepY) {
    for (let x = left; x < right; x += stepX) {
      const i = y * stride + x * channels;
      seen.add((pixels[i] << 16) | (pixels[i + 1] << 8) | pixels[i + 2]);
    }
  }
  return seen.size;
}

/** Has the standalone opened a window yet? */
function windowIsUp() {
  if (process.platform === 'linux') {
    const found = sh('xdotool', ['search', '--name', WINDOW_TITLE]);
    return found.status === 0 && (found.stdout ?? '').trim().length > 0;
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

/** Capture the plugin's window alone. */
function captureWindow() {
  if (process.platform === 'linux') {
    const found = sh('xdotool', ['search', '--name', WINDOW_TITLE]);
    const id = (found.stdout ?? '').trim().split('\n').filter(Boolean).pop();
    if (!id) return { status: 1, stderr: `xdotool found no window titled "${WINDOW_TITLE}"` };
    return sh('import', ['-window', id, output]);
  }

  if (process.platform === 'darwin') {
    // `-l <CGWindowID>` reads the window's own buffer, so it does not matter
    // where the window sits or how small the runner's display is. `-R` takes a
    // screen *region* and silently loses anything hanging off the edge, which is
    // how an earlier job cropped the right rail away and still passed.
    //
    // Numeric constants rather than `$.kCGWindowListOptionOnScreenOnly`: JXA
    // does not bridge every CoreGraphics enum and an undefined one becomes 0,
    // which asks for a different window list and returns nothing.
    const lookup = `
      ObjC.import('CoreGraphics');
      var found = '';
      var windows = $.CGWindowListCopyWindowInfo(1 | 16, 0);
      var count = windows.count;
      for (var i = 0; i < count && !found; i++) {
        var w = windows.objectAtIndex(i);
        var owner = String(ObjC.unwrap(w.objectForKey('kCGWindowOwnerName')) || '');
        var bounds = w.objectForKey('kCGWindowBounds');
        var width = bounds ? Number(ObjC.unwrap(bounds.objectForKey('Width'))) : 0;
        if (owner.toLowerCase().indexOf('${PROCESS_NAME}') !== -1 && width > 200) {
          found = String(ObjC.unwrap(w.objectForKey('kCGWindowNumber')));
        }
      }
      found;
    `;
    const found = sh('osascript', ['-l', 'JavaScript', '-e', lookup]);
    const id = (found.stdout ?? '').trim();
    if (/^\d+$/.test(id)) {
      // `-o` drops the drop-shadow so the image is the window and nothing else.
      return sh('screencapture', ['-x', '-o', '-l', id, output]);
    }

    // No window id: fall back to a region grab of the window's bounds.
    //
    // ⛔ Not a nicety. `CGWindowListCopyWindowInfo` returns nothing to
    // `osascript` on GitHub's macOS image, which does not grant it the Screen
    // Recording permission `screencapture` itself has. Without this fallback the
    // job would fail on a *permission* and tell us nothing about whether the
    // editor renders — an uninformative red is worse than a partial picture.
    // The inset colour count still refuses a blank window.
    const bounds = sh('osascript', [
      '-e',
      `tell application "System Events"
         set procs to (every process whose name contains "${PROCESS_NAME}")
         if (count of procs) = 0 then return "none"
         set p to item 1 of procs
         if (count of windows of p) = 0 then return "none"
         set w to window 1 of p
         set position of w to {0, 25}
         set {x, y} to position of w
         set {ww, hh} to size of w
         return (x as text) & "," & (y as text) & "," & (ww as text) & "," & (hh as text)
       end tell`,
    ]);
    const answer = (bounds.stdout ?? '').trim();
    if (!answer || answer === 'none') {
      return {
        status: 1,
        stderr:
          `no CGWindowID (${found.stderr || 'no output'}) and no window bounds ` +
          `either (${bounds.stderr || 'no output'})`,
      };
    }

    console.log(
      `::warning::macOS capture is a region grab, not a window buffer (no CGWindowID)`,
    );
    const [x, y, w, h] = answer.split(',').map(Number);
    return sh('screencapture', ['-x', '-R', `${x},${y},${w},${h}`, output]);
  }

  // Windows: PrintWindow, which asks the window to render *itself* into a
  // bitmap. `CopyFromScreen` lies whenever the window is larger than the
  // display — the runner's desktop is smaller than this window, so a screen grab
  // would clip the right rail off and photograph the taskbar in its place.
  //
  // PW_RENDERFULLCONTENT (2) is what makes this work for a WebView2 child
  // surface; without it the client area comes back blank.
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
    if (-not $p) { Write-Error 'no standalone process with a window'; exit 1 }
    $handle = $p.MainWindowHandle
    [Cap]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 800
    $r = New-Object Cap+RECT
    [Cap]::GetWindowRect($handle, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $h = $r.B - $r.T
    if ($w -le 0 -or $h -le 0) { Write-Error "bad window rect \${w}x\${h}"; exit 1 }

    # ⛔ PrintWindow can return $true and still hand back an empty bitmap.
    # WebView2 composites its content on the GPU into a separate surface, so the
    # window has nothing for GDI to copy — measured here as 5 distinct colours on
    # a 1240x804 window that was rendering the whole UI perfectly on screen.
    # Trusting the return value alone is how this gate would have reported a
    # working editor as broken on every Windows build.
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
    if (-not $printed -or (Distinct-Colours $bmp) -lt 16) {
      $g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size($w, $h)))
      $how = 'CopyFromScreen (PrintWindow came back blank)'
    }

    $bmp.Save('${output.replace(/\\/g, '\\\\')}', [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Output "captured \${w}x\${h} via $how"
  `;
  return sh('powershell', ['-NoProfile', '-Command', ps]);
}

if (!existsSync(binary)) {
  die(
    `no standalone binary at ${binary} — build it first ` +
      '(cargo build -p freally-midi-master-plugin --release)',
  );
}

// `dummy` rather than `auto`: a CI runner has no sound card and no JACK server,
// and this gate is about the editor rather than the audio path. Naming the
// backend keeps a missing device from looking like a missing window.
const app = spawn(binary, ['--backend', 'dummy'], {
  stdio: ['ignore', 'inherit', 'inherit'],
  env: { ...process.env },
});

let exitedEarly = null;
app.on('exit', (code) => {
  exitedEarly = code;
});

try {
  console.log(`waiting for the plugin window on ${process.platform}…`);
  const started = Date.now();
  let up = false;
  while (!up && Date.now() - started < WINDOW_TIMEOUT_MS) {
    if (exitedEarly !== null) {
      die(`the standalone exited before opening a window (code ${exitedEarly})`);
    }
    await sleep(2_000);
    up = windowIsUp();
  }
  if (!up) {
    die(
      `no window titled "${WINDOW_TITLE}" appeared within ${WINDOW_TIMEOUT_MS / 60000} minutes`,
    );
  }

  console.log('window is up; letting it paint…');
  await sleep(PAINT_MS);

  const shot = captureWindow();
  if (shot.status !== 0) {
    die(`window capture failed: ${shot.stderr || shot.stdout || 'no output'}`);
  }
  if (!existsSync(output)) {
    die('the capture tool reported success but wrote no file');
  }

  const png = readFileSync(output);
  const { size } = statSync(output);
  const width = png.readUInt32BE(16);
  const height = png.readUInt32BE(20);
  const colours = distinctColours(png);

  console.log(
    `captured ${width}x${height}, ${Math.round(size / 1024)} KB, ${colours ?? '?'} distinct colours`,
  );

  if (width < 800 || height < 500) {
    die(`the capture is ${width}x${height} — too small to be the plugin window`);
  }
  if (colours === null) {
    die('the capture could not be decoded, so it is not evidence of a rendered window');
  }
  // ⛔ The assertion this whole file exists for. Everything above passes with an
  // empty window.
  if (colours < MIN_COLOURS) {
    die(
      `the window rendered ${colours} distinct colours (needs ${MIN_COLOURS}) — ` +
        'it opened but drew nothing, which is the editor failing, not passing',
    );
  }
} finally {
  app.kill();
  if (isWindows) {
    spawnSync('taskkill', ['/F', '/IM', `${PROCESS_NAME}.exe`], { stdio: 'ignore' });
  } else {
    spawnSync('pkill', ['-f', PROCESS_NAME], { stdio: 'ignore' });
  }
}
