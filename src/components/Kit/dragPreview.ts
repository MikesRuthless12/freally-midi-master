/**
 * Drawing the picture that rides on the cursor, and getting it to the plugin
 * (TASK-063C).
 *
 * ⛔ **This is NOT `event.dataTransfer.setDragImage`.** That draws inside the
 * webview and disappears the instant the cursor crosses into the DAW — which is
 * the only place the producer needs to see it. The pixels go down to
 * `plugin/src/drag/windows.rs`, which hands them to the shell's own drag helper
 * so the image follows the cursor across every window on the desktop.
 *
 * `previewLayout.ts` holds every measurement and is tested there; this file is
 * the canvas calls and the encode.
 */

import {
  allNotes,
  patternTicks,
  previewBars,
  songClips,
  PREVIEW_HEIGHT,
  PREVIEW_LABEL_HEIGHT,
  PREVIEW_WIDTH,
} from './previewLayout';
import { readPalette, type Palette } from '../PianoRoll/palette';
import type { Lane, Pattern, Song } from '../../lib/ipc-types';
import type { DragFormat } from '../../state/drag';

/** What `drag_start` accepts, and what `drag::Preview` expects on the other side. */
export type PreviewPayload = { width: number; height: number; rgba: string };

/**
 * Base64 without blowing the stack.
 *
 * ⛔ `String.fromCharCode(...bytes)` on a hundred thousand pixels throws
 * `RangeError: Maximum call stack size exceeded` — the argument list is the
 * limit, not the string. Chunked, and the chunk is well inside every engine's
 * bound.
 */
export function toBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000;
  let binary = '';
  for (let at = 0; at < bytes.length; at += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(at, at + CHUNK));
  }
  return btoa(binary);
}

/**
 * Draw what is about to be dragged, and encode it for the bridge.
 *
 * Returns `null` when there is nothing to draw or no canvas to draw on — ⚠ **a
 * missing picture must never stop the drag.** A producer would far rather move
 * a file with a plain cursor than not move it at all, and the plugin treats the
 * preview as optional on every path for the same reason.
 */
export function drawDragPreview(
  patterns: Pattern[],
  label: string,
  lane?: Lane,
): PreviewPayload | null {
  const first = patterns[0];
  if (!first) return null;
  // ⛔ **`lane` narrows the picture to what will actually be dropped.** A
  // per-lane row sends the pattern whole and names the lane — the plugin does
  // the cutting — so without this the drag image showed the whole kit and every
  // lane chip drew the same picture.
  const marks = previewBars(allNotes(patterns, lane), patternTicks(first));
  return draw(label, (ctx, palette) => {
    ctx.fillStyle = palette.primary;
    for (const mark of marks) {
      ctx.globalAlpha = mark.alpha;
      ctx.fillRect(mark.x, mark.y, mark.width, mark.height);
    }
  });
}

/**
 * The arrangement as it will land in the DAW (TASK-144).
 *
 * ⛔⛔ **This used to be the "purple graph" Mike objected to** — one card with a
 * row of density bars, drawn identically whether the drop would land 1 file or
 * 13, saying nothing about MIDI or audio and nothing about the modifier. His
 * correction names what it has to be instead: *"i want the song arrangement
 * being dragged in to actually show the midi clips either together back to back
 * or stacked with the 'Ctrl' or 'Command' keys being pressed, same with audio
 * clips."* A producer looking at the cursor should recognise the shape of their
 * own record and see where it is going to sit.
 *
 * ⚠ **`stacked` is read once, when the gesture becomes a drag.** The bitmap
 * goes to the shell before `DoDragDrop` and cannot be redrawn mid-gesture — see
 * [`songClips`]. The payload still swaps as the modifier changes; only the
 * picture is fixed.
 *
 * ⚠ **`format` and `stacked` are REQUIRED, with no default between them.** A
 * default of `'midi'` would hand an audio drop a MIDI picture the moment a
 * caller forgot the argument — silently, and looking correct — which is the
 * readout-that-lies case this file is otherwise built around. The one call site
 * has both in scope.
 */
export function drawSongPreview(
  song: Song,
  label: string,
  format: DragFormat,
  stacked: boolean,
): PreviewPayload | null {
  const clips = songClips(song, stacked);
  return draw(label, (ctx, palette) => {
    // ⛔ **A clip is drawn as a clip**: a filled, bordered rectangle at its true
    // bar and its true length. That is the whole of Mike's correction, and it is
    // the same shape `SongTimeline` paints, so the cursor and the timeline agree.
    //
    // ⚠ **MIDI and audio are told apart by FILL, not by a word.** He names them
    // as two kinds of chip; there is no room for a legend at this size. MIDI is
    // the solid primary the note blocks already use; audio is hollow with a
    // centre line, which is what a waveform reduces to at four pixels tall.
    ctx.globalAlpha = 0.85;
    ctx.fillStyle = palette.primary;
    ctx.strokeStyle = palette.primary;
    ctx.lineWidth = 1;
    for (const clip of clips) {
      if (format === 'audio') {
        ctx.strokeRect(
          clip.x + 0.5,
          clip.y + 0.5,
          Math.max(1, clip.width - 1),
          clip.height - 1,
        );
        ctx.fillRect(clip.x, clip.y + clip.height / 2 - 0.5, clip.width, 1);
      } else {
        ctx.fillRect(clip.x, clip.y, clip.width, clip.height);
      }
    }
  });
}

/**
 * The card, the name, and whatever the caller paints under it.
 *
 * ⚠ **A paint hook rather than two payload parameters.** This took a
 * `PreviewBar[]` *and* a `PreviewClip[]` *and* a format flag, where every call
 * passed one list and an empty sentinel for the other. What is genuinely shared
 * is the card, the label, the divider and the encode; how the marks are painted
 * belongs to whoever knows what they mean.
 */
function draw(
  label: string,
  paint: (ctx: CanvasRenderingContext2D, palette: Palette) => void,
): PreviewPayload | null {
  const canvas = document.createElement('canvas');
  canvas.width = PREVIEW_WIDTH;
  canvas.height = PREVIEW_HEIGHT;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;

  // ⛔ **The app's own palette, read once.** A private copy of these four tokens
  // with its own fallback hexes had already drifted from `palette.ts` at birth —
  // four different colours — which `palette.ts`'s header records happening
  // before: "when each had its own `Palette` type and its own `readPalette` the
  // three had already drifted".
  const palette = readPalette(document.documentElement);

  ctx.fillStyle = palette.surface2;
  ctx.fillRect(0, 0, PREVIEW_WIDTH, PREVIEW_HEIGHT);
  ctx.strokeStyle = palette.border;
  ctx.lineWidth = 1;
  ctx.strokeRect(0.5, 0.5, PREVIEW_WIDTH - 1, PREVIEW_HEIGHT - 1);

  // The name, which is the half that says *which* loop this is. Clipped rather
  // than wrapped: a stem name is one line by construction, and a second line
  // would eat the drawing below it.
  ctx.save();
  ctx.beginPath();
  ctx.rect(8, 0, PREVIEW_WIDTH - 16, PREVIEW_LABEL_HEIGHT);
  ctx.clip();
  ctx.fillStyle = palette.text;
  ctx.font = '12px system-ui, sans-serif';
  ctx.textBaseline = 'middle';
  ctx.fillText(label, 8, PREVIEW_LABEL_HEIGHT / 2);
  ctx.restore();

  ctx.beginPath();
  ctx.moveTo(0, PREVIEW_LABEL_HEIGHT);
  ctx.lineTo(PREVIEW_WIDTH, PREVIEW_LABEL_HEIGHT);
  ctx.stroke();

  paint(ctx, palette);

  let pixels: ImageData;
  try {
    pixels = ctx.getImageData(0, 0, PREVIEW_WIDTH, PREVIEW_HEIGHT);
  } catch {
    // ⚠ The one guard that genuinely fires: jsdom has a 2D context that draws
    // nothing and refuses to read back. The drag goes ahead without a picture
    // rather than not at all.
    return null;
  }
  return {
    width: PREVIEW_WIDTH,
    height: PREVIEW_HEIGHT,
    rgba: toBase64(new Uint8Array(pixels.data.buffer)),
  };
}
