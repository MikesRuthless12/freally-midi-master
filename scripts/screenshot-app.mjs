#!/usr/bin/env node
/**
 * Launch the real app and photograph it.
 *
 * CI runs this on all three OSes so a human can look at three PNGs and see the
 * Studio actually rendering, rather than inferring it from a green tick. It
 * catches the class of failure every other gate misses: the app compiles, the
 * tests pass, and the window comes up blank or half-drawn.
 *
 * Deliberately screenshots the **native window**, not a browser. Playwright
 * already covers the DOM; what is unproven is that the WebView paints it
 * inside a real Tauri window on that platform.
 *
 * Usage: node scripts/screenshot-app.mjs <output.png>
 */

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(process.argv[2] ?? 'app-screenshot.png');
mkdirSync(dirname(output), { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function die(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

/** Wait until something is listening on the Vite port. */
async function waitForDevServer(timeoutMs = 240_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    try {
      const res = await fetch('http://localhost:1420/');
      if (res.ok) return true;
    } catch {
      // not up yet
    }
    await sleep(2_000);
  }
  return false;
}

function capture() {
  if (process.platform === 'darwin') {
    // -x suppresses the shutter sound; the runner has a real window server.
    return spawnSync('screencapture', ['-x', output], { encoding: 'utf8' });
  }
  if (process.platform === 'linux') {
    // ImageMagick against the Xvfb root window.
    return spawnSync('import', ['-window', 'root', output], { encoding: 'utf8' });
  }
  // Windows: capture the virtual screen via .NET.
  const ps = `
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save('${output.replace(/\\/g, '\\\\')}', [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
  `;
  return spawnSync('powershell', ['-NoProfile', '-Command', ps], { encoding: 'utf8' });
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
  console.log('waiting for the dev server…');
  if (!(await waitForDevServer())) {
    die('the dev server never came up — the app cannot have rendered');
  }
  console.log('dev server up; waiting for the window to build and paint…');

  // The Rust side still has to compile and open a window after Vite is ready.
  // Poll rather than guess, but keep a floor so we never shoot a blank frame.
  await sleep(45_000);

  if (exitedEarly !== null) {
    die(`the app exited before it could be photographed (code ${exitedEarly})`);
  }

  const shot = capture();
  if (shot.status !== 0) {
    die(`screen capture failed: ${shot.stderr || shot.stdout || 'no output'}`);
  }
  if (!existsSync(output)) {
    die('the capture tool reported success but wrote no file');
  }

  // A capture of a dead session is a few hundred bytes of black. Treating that
  // as a pass would make this whole job decorative.
  const { size } = statSync(output);
  if (size < 10_000) {
    die(`the screenshot is only ${size} bytes — almost certainly a blank screen`);
  }

  console.log(`captured ${output} (${Math.round(size / 1024)} KB)`);
} finally {
  app.kill();
  // Vite and the app are separate processes on some platforms.
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/F', '/IM', 'freally-midi-master.exe'], { stdio: 'ignore' });
  } else {
    spawnSync('pkill', ['-f', 'freally-midi-master'], { stdio: 'ignore' });
  }
}
