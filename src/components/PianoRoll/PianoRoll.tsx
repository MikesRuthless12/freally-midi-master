import { memo, useCallback, useEffect, useMemo, useReducer, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
import { useEditing } from '../../state/editing';
import { useCanvasPalette } from './palette';
import type { Lane, Note, Pattern } from '../../lib/ipc-types';
import {
  addNote,
  addedIds,
  barTicks,
  deleteNotes,
  laneOf,
  clampedMove,
  moveNotes,
  noteAt,
  noteId,
  notesInBox,
  notesOf,
  patternTicks,
  resizeNotes,
  snapTick,
  stepTicks,
  type MelodicPart,
  type NoteId,
} from './notes';
import {
  MIN_NOTE_WIDTH,
  edgeAt,
  isBlackKey,
  pitchName,
  pitchToY,
  rowsFor,
  rowY,
  tickToX,
  ticksToWidth,
  xToTick,
  yToPitch,
  type View,
} from './geometry';
import { loadScale, scaleClasses } from './scale';
import { RollBar } from './RollBar';
import { Ruler } from './Ruler';
import { TransformMenu } from './TransformMenu';
import { VelocityLane } from './VelocityLane';
import { reselect } from './transforms';
import { stemId } from './velocity';
import { auditionNote } from './audition';
import { useRollShortcuts } from './shortcuts';
import './PianoRoll.css';

/**
 * The piano roll (TASK-041).
 *
 * ⛔ **Canvas rather than DOM, and that is a requirement rather than a
 * preference.** `DrumGrid` draws ~1,150 spans for an 8-bar kit and that is
 * already the most expensive thing on the page; a roll is 128 rows deep, so the
 * same approach would be ~15,000 elements before a single note. The roadmap asks
 * for 60 fps pan and zoom, and no DOM tree of that size delivers it.
 *
 * ⛔ **The cost of canvas is that nothing in it is in the accessibility tree or
 * reachable by a test.** So the notes are *also* published as a visually-hidden
 * list below. That is not a test hack bolted on: a canvas with no text
 * alternative is inaccessible, and one that Playwright can only poke at by
 * coordinate cannot assert "the note moved", which is exactly what the roadmap
 * says these tests must assert. One structure answers both.
 */

/** How wide the keyboard gutter is drawn, in CSS pixels. */
const GUTTER = 56;

/**
 * A CSS-pixel value snapped to the device's own pixel grid (TASK-041F).
 *
 * The canvas is scaled by `devicePixelRatio`, so a hairline at a fractional CSS
 * x lands between two device columns and is drawn as two half-lit ones. That is
 * the difference between a ruler and a smudge, and it gets worse — not better —
 * on the high-DPR displays the plugin is most often opened on.
 */
function crisp(value: number, dpr: number): number {
  return Math.round(value * dpr) / dpr;
}

/**
 * A note the user is previewing mid-drag, or the note itself when idle.
 *
 * ⛔ **The delta arrives already clamped**, by `clampedMove`/`clampedResize` in
 * `notes.ts` — the same functions the commit uses. This applied the raw delta,
 * so dragging a chord toward the clip's end or toward pitch 127 drew one thing
 * and committed another: the preview kept sliding while the commit had already
 * stopped, and the notes landed where the canvas had stopped showing them.
 */
function previewNote(
  note: Note,
  drag: ReturnType<typeof useEditing.getState>['drag'],
  selected: boolean,
  lane: Lane,
): Note {
  if (drag === null) return note;
  // ⛔ Ahead of the selection guard: a paint across the velocity lane writes to
  // whatever it passes over, selected or not, and the note body has to re-shade
  // live as the cap moves or the lane is editing something invisible.
  if (drag.kind === 'velocity') {
    const vel = drag.values[stemId(lane, note)];
    return vel === undefined ? note : { ...note, vel };
  }
  if (!selected) return note;
  if (drag.kind === 'move') {
    return {
      ...note,
      startTick: note.startTick + drag.deltaTicks,
      pitch: note.pitch + drag.deltaPitch,
    };
  }
  if (drag.kind === 'resize') {
    return drag.edge === 'start'
      ? {
          ...note,
          startTick: note.startTick + drag.deltaTicks,
          lenTicks: note.lenTicks - drag.deltaTicks,
        }
      : { ...note, lenTicks: note.lenTicks + drag.deltaTicks };
  }
  return note;
}

/**
 * The canvas's text alternative, and the seam every Playwright assertion about
 * note data goes through.
 *
 * ⛔ **Memoised, and that is not a micro-optimisation.** The roll subscribes to
 * the live drag, so it re-renders on every pointermove and on every 30 Hz
 * playhead tick — and this list walks every note calling an i18next
 * interpolation for each one. None of that work can change anything: a drag is
 * a delta held outside the pattern until pointerup, so the notes are identical
 * on every frame of it. Rebuilding this per frame is precisely the DOM cost the
 * canvas exists to avoid.
 */
const NoteList = memo(function NoteList({
  notes,
  selection,
}: {
  notes: readonly Note[];
  selection: ReadonlySet<NoteId>;
}) {
  const { t } = useTranslation();
  return (
    <ul className="visually-hidden" data-testid="roll-notes" aria-label={t('roll.notesLabel')}>
      {notes.map((note) => {
        const id = noteId(note);
        return (
          <li
            key={id}
            data-note={id}
            data-tick={note.startTick}
            data-pitch={note.pitch}
            data-len={note.lenTicks}
            data-vel={note.vel}
            data-selected={selection.has(id) || undefined}
          >
            {t('roll.noteDescription', {
              pitch: pitchName(note.pitch),
              tick: note.startTick,
              velocity: note.vel,
            })}
          </li>
        );
      })}
    </ul>
  );
});

export function PianoRoll({
  pattern,
  part,
  playhead,
}: {
  pattern: Pattern;
  part: MelodicPart;
  playhead: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const paletteOf = useCanvasPalette(wrapRef);
  /** Ask for a repaint from something that is not a store change. */
  const [, bump] = useReducer((n: number) => n + 1, 0);

  const editPattern = useSession((s) => s.editPattern);
  const pins = useSession((s) => s.pins);
  const snap = useEditing((s) => s.snap);
  const zoom = useEditing((s) => s.zoom);
  const rowHeight = useEditing((s) => s.rowHeight);
  const scrollTick = useEditing((s) => s.scrollTick);
  const topPitch = useEditing((s) => s.topPitch);
  const selection = useEditing((s) => s.selection);
  const drag = useEditing((s) => s.drag);
  const foldToScale = useEditing((s) => s.foldToScale);
  const foldToNotes = useEditing((s) => s.foldToNotes);
  const fitTo = useEditing((s) => s.fitTo);

  // ⛔⛔ **Show the whole clip when the clip changes** (Mike, 2026-08-09: *"the
  // entire midi piano roll pattern needs to be shown … whether it's 4 or 8 bars
  // shouldn't matter"*). The roll's zoom was a fixed 64px per quarter, so
  // anything past four bars ran off the right-hand edge of the stage and the
  // producer had to zoom out by hand after every generation.
  //
  // ⚠ Keyed on the clip's **id and length**, not on every render: re-fitting
  // while someone is zooming would fight them for the control, and re-fitting on
  // each edit would snap the view back mid-gesture. A new take or a different
  // length is exactly when a fresh view is wanted, and nowhere else.
  //
  // ⚠ The lint rule wants the whole `pattern` in the deps and it is wrong here:
  // that object is rebuilt on every edit, so honouring it would re-fit the view
  // on each note a producer moves. The three fields below are the only ones this
  // effect reads, and they are the ones that mean "a different clip".
  const totalTicks = patternTicks(pattern);
  const ppq = pattern.ppq;
  useEffect(() => {
    const wrap = wrapRef.current;
    if (wrap === null) return;
    fitTo(totalTicks, ppq, wrap.clientWidth - GUTTER);
  }, [pattern.id, totalTicks, ppq, fitTo]);

  const lane = laneOf(part);
  const notes = useMemo(() => notesOf(pattern, lane), [pattern, lane]);

  // Selection and note editing (TASK-041A). Mounted only while a melodic tab is
  // showing its own pattern, which is exactly when these keys should be live.
  useRollShortcuts(pattern, lane, editPattern, playhead * patternTicks(pattern));

  /**
   * Frame the clip's own register when a *different* clip arrives.
   *
   * ⛔ **Without this the roll opens on an empty stretch of keyboard.** A
   * generated melody sits around MIDI 54–66 and the default top pitch is 84, so
   * at a 20 px row height the notes are several screens below the fold — the
   * producer sees a blank grid and reasonably concludes the generator wrote
   * nothing. That is the same "indistinguishable from a failure" trap
   * `CenterStage` avoids by not drawing an empty grid at all.
   *
   * ⛔ **It sets the row height too, which is the half that was missing.**
   * Scrolling to the register only ever showed as many semitones as the current
   * row height happened to allow; a two-octave melody was still cropped at both
   * ends. `frameTo` is to the vertical what `fitTo` is to the horizontal.
   *
   * ⛔ **Keyed on the pattern's id, not on the pattern object.** Every note edit
   * produces a new object; re-framing on those would yank the view back mid-edit
   * every time a note was dragged, which is unusable. A new id means a new
   * generation, which is the only time the register can have moved.
   */
  const framedFor = useRef<string | null>(null);
  useEffect(() => {
    if (framedFor.current === pattern.id) return;
    framedFor.current = pattern.id;
    if (notes.length === 0) return;

    const height = wrapRef.current?.clientHeight ?? 0;
    const pitches = notes.map((note) => note.pitch);
    useEditing.getState().frameTo(Math.max(...pitches), Math.min(...pitches), height);
  }, [pattern.id, notes]);

  // The two pixels the renderer floors a note at, in this view's own ticks —
  // so a note is clickable over exactly the span it is drawn over.
  const minHitTicks = (MIN_NOTE_WIDTH / zoom) * pattern.ppq;

  const view: View = useMemo(
    () => ({ zoom, rowHeight, scrollTick, topPitch, ppq: pattern.ppq }),
    [zoom, rowHeight, scrollTick, topPitch, pattern.ppq],
  );

  /**
   * The gesture's starting point, kept in a ref rather than in state.
   *
   * ⛔ A pointermove must not re-render to be handled — that is the whole reason
   * the drag delta lives in a store the canvas reads imperatively. Putting the
   * anchor in React state would render on every frame of every drag, which is
   * the cost this component is built to avoid.
   */
  const anchor = useRef<{ tick: number; pitch: number } | null>(null);

  /**
   * The note to reduce the selection to on pointerup, if the gesture turns out
   * to have been a click rather than a drag.
   *
   * A ref for the same reason as `anchor`: it is decided on pointerdown, read
   * once on pointerup, and nothing between them should render because of it.
   */
  const collapseTo = useRef<string | null>(null);

  /**
   * Whether the pointer actually moved during this gesture.
   *
   * ⚠ **Not the same question as "did the notes move".** A drag against the
   * start of the pattern or the edge of the register is clamped to a zero
   * delta, and reading the delta made those gestures indistinguishable from a
   * click. What decides whether a gesture was a click is the pointer.
   */
  const moved = useRef(false);

  /**
   * The note under the pointer, for the hover affordance (TASK-041F).
   *
   * In a ref rather than in state, and it asks for a repaint only when the
   * *identity* changes — a pointermove across one note fires dozens of times
   * and none of them can alter what is drawn.
   */
  const hovered = useRef<NoteId | null>(null);

  // ---- scale awareness (TASK-041B) --------------------------------------

  /**
   * The key the rows are tinted against — the pin if the producer set one, and
   * the pattern's own otherwise.
   *
   * ⛔ **The same resolution the scale chip uses**, because the roll's picker
   * writes through to `pins.scale` rather than keeping a second copy. Tinting
   * against `pattern.scale` while the picker showed the pin would put the two
   * halves of one control into disagreement: the header would say Lydian and
   * the rows would still be shaded for the minor the last Generate ran in.
   */
  const scale = pins.scale ?? pattern.scale;
  const keyRoot = pins.keyRoot ?? pattern.keyRoot;

  // Asked once per scale; the draw reads the cache synchronously and simply
  // does not tint until it lands.
  useEffect(() => {
    void loadScale(scale).then(bump);
  }, [scale, bump]);

  const classes = scaleClasses(scale, keyRoot);
  const written = useMemo(() => new Set(notes.map((note) => note.pitch)), [notes]);
  const folded = foldToScale || foldToNotes;

  /**
   * Which rows survive the folds.
   *
   * ⛔ **A row holding a note is always kept, even when it is out of key.** The
   * generators write chromatic passing tones on purpose, and `Fold to scale`
   * hiding one would make an audible note invisible — the roadmap's own
   * verification for this task is that a folded out-of-scale note is still
   * audible and still exported. Keeping its row is the honest way to satisfy
   * that: the fold tidies the *empty* rows, which is what it is for.
   */
  const keepRow = useCallback(
    (pitch: number) => {
      if (written.has(pitch)) return true;
      if (foldToNotes) return false;
      if (foldToScale && classes !== null) return classes.has(pitch % 12);
      return true;
    },
    [written, foldToNotes, foldToScale, classes],
  );

  // ---- drawing ----------------------------------------------------------

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    const ctx = canvas.getContext('2d');
    if (ctx === null) return;

    const width = wrap.clientWidth;
    const height = wrap.clientHeight;
    if (width === 0 || height === 0) return;

    // ⛔ devicePixelRatio-aware, re-read on every draw rather than cached. The
    // plugin window can be dragged between a scaled and an unscaled monitor
    // while it is open, and a canvas backing store sized for the old ratio is
    // the blurry-editor bug that has no other symptom.
    const dpr = window.devicePixelRatio || 1;
    if (
      canvas.width !== Math.round(width * dpr) ||
      canvas.height !== Math.round(height * dpr)
    ) {
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    // ⛔ **Read once per theme, not once per frame.** `getComputedStyle` forces
    // a synchronous style recalculation, and this runs from a rAF scheduled
    // right after a React commit — when the style tree is dirty — so it was the
    // single most expensive thing in the draw, seven property reads deep, to
    // fetch values that only move when the theme does.
    const palette = paletteOf();
    if (palette === null) return;
    const rollWidth = width - GUTTER;

    ctx.fillStyle = palette.surface;
    ctx.fillRect(0, 0, width, height);

    const rows = rowsFor(view, height, keepRow);

    // Row banding, in three states rather than two (TASK-041B): a black key is
    // darker, a row *outside the key* is dimmed further, and the root row is
    // marked. That is the difference between a roll that shows you a keyboard
    // and one that shows you the key you are writing in.
    rows.pitches.forEach((pitch, index) => {
      const y = rowY(index, view);
      const inKey = classes === null || classes.has(pitch % 12);
      const isRoot = classes !== null && pitch % 12 === keyRoot % 12;

      ctx.fillStyle = isBlackKey(pitch) ? palette.surface2 : palette.surface;
      ctx.fillRect(GUTTER, y, rollWidth, rowHeight);

      if (!inKey) {
        // Dimmed rather than hidden: an out-of-key row is still writable, and
        // the generators use chromatic passing tones on purpose.
        ctx.globalAlpha = 0.55;
        ctx.fillStyle = palette.surface2;
        ctx.fillRect(GUTTER, y, rollWidth, rowHeight);
        ctx.globalAlpha = 1;
      } else if (isRoot) {
        ctx.globalAlpha = 0.16;
        ctx.fillStyle = palette.primary;
        ctx.fillRect(GUTTER, y, rollWidth, rowHeight);
        ctx.globalAlpha = 1;
      }

      // An octave line, so counting rows is never necessary. Drawn under a fold
      // too, where it is the only thing left saying how far the eye has moved.
      if (pitch % 12 === 0) {
        ctx.fillStyle = palette.border;
        ctx.fillRect(GUTTER, y + rowHeight - 1, rollWidth, 1);
      }
    });

    // Vertical grid. Beats are drawn from the snap value so the lines a note
    // lands on are the lines that are visible — a grid showing 16ths while
    // snapping to triplets is a ruler that lies.
    const step = stepTicks(snap, pattern.ppq);
    const beat = pattern.ppq;
    const bar = barTicks(pattern);
    const totalTicks = patternTicks(pattern);
    ctx.save();
    ctx.beginPath();
    ctx.rect(GUTTER, 0, rollWidth, height);
    ctx.clip();
    for (let tick = Math.floor(scrollTick / step) * step; ; tick += step) {
      const x = GUTTER + tickToX(tick, view);
      if (x > width) break;
      if (tick > totalTicks) break;
      const isBar = tick % bar === 0;
      const isBeat = tick % beat === 0;
      // Sub-beat lines are skipped once they would be closer than three pixels:
      // below that they stop reading as a grid and start reading as noise.
      if (!isBeat && ticksToWidth(step, view) < 3) continue;
      // Snapped to a device pixel, for the reason on the playhead below: a
      // 1 px line at a fractional x is drawn as two half-lit columns, and a
      // grid of those reads as haze rather than as a grid.
      ctx.fillStyle = isBar ? palette.text3 : palette.border;
      ctx.fillRect(crisp(x, dpr), 0, Math.max(1 / dpr, crisp(isBar ? 1.5 : 1, dpr)), height);
    }

    // Past the end of the clip is not editable space, and it says so.
    const endX = GUTTER + tickToX(totalTicks, view);
    if (endX < width) {
      ctx.fillStyle = palette.surface2;
      ctx.globalAlpha = 0.6;
      ctx.fillRect(endX, 0, width - endX, height);
      ctx.globalAlpha = 1;
    }

    // Notes.
    for (const note of notes) {
      // ⛔ Culled before the id string is built. `noteId` allocates a template
      // string per note per frame, and most notes are off-screen at any useful
      // zoom — so this was thousands of throwaway strings a second to decide
      // the colour of something that was never drawn.
      //
      // ⛔ `keepRow` always keeps a row that holds a note, so a *written* note
      // can never be folded out of view. That is what makes "a hidden row still
      // plays and still exports" true by construction rather than by a marker
      // drawn to apologise for it. The `null` here is therefore only ever a
      // note scrolled off the top or bottom.
      if (pitchToY(note.pitch, rows, view) === null) continue;

      const id = noteId(note);
      const selected = selection.size > 0 && selection.has(id);
      const shown = previewNote(note, drag, selected, lane);
      // Mid-drag the preview can sit on a row the fold removed, which is a
      // real state: dragging a note out of the key with `Fold to scale` on.
      // It is drawn only where there is a row for it; pointerup re-runs
      // `keepRow` with the note in its new place and the row comes back.
      const shownY = pitchToY(shown.pitch, rows, view);
      if (shownY === null) continue;
      const x = GUTTER + tickToX(shown.startTick, view);
      const w = Math.max(2, ticksToWidth(shown.lenTicks, view));
      if (x > width || x + w < GUTTER) continue;

      // Velocity maps to fill opacity, so a written accent pattern is legible
      // without opening the velocity lane.
      ctx.globalAlpha = 0.45 + (shown.vel / 127) * 0.55;
      ctx.fillStyle = selected ? palette.signal : palette.primary;
      ctx.fillRect(x, shownY + 1, w, rowHeight - 2);
      ctx.globalAlpha = 1;

      // Decided once per theme in `readPalette`, not per note per frame.
      const outline = selected ? palette.outlineOnSignal : palette.outlineOnPrimary;

      // The hover affordance: an outline on the note the pointer is over, and
      // nothing on the ninety-nine it is not (TASK-041F). Drawn at a lower
      // weight than the selection's, so the two read as "reachable" and
      // "chosen" rather than as the same state twice.
      if (!selected && hovered.current !== null && hovered.current === id) {
        ctx.globalAlpha = 0.5;
        ctx.strokeStyle = outline;
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 0.5, shownY + 1.5, w - 1, rowHeight - 3);
        ctx.globalAlpha = 1;
      }

      if (selected) {
        ctx.strokeStyle = outline;
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 0.5, shownY + 1.5, w - 1, rowHeight - 3);
      }
    }

    // Marquee. Drawn from row indices so it tracks a folded roll, where the
    // pitches it spans are not contiguous.
    if (drag?.kind === 'marquee') {
      const x1 = GUTTER + tickToX(Math.min(drag.fromTick, drag.toTick), view);
      const x2 = GUTTER + tickToX(Math.max(drag.fromTick, drag.toTick), view);
      const a = rows.indexOf.get(drag.fromPitch) ?? 0;
      const b = rows.indexOf.get(drag.toPitch) ?? rows.pitches.length - 1;
      const y1 = rowY(Math.min(a, b), view);
      const y2 = rowY(Math.max(a, b), view) + rowHeight;
      ctx.strokeStyle = palette.signal;
      ctx.lineWidth = 1;
      ctx.strokeRect(x1 + 0.5, y1 + 0.5, x2 - x1, y2 - y1);
      ctx.globalAlpha = 0.15;
      ctx.fillStyle = palette.signal;
      ctx.fillRect(x1, y1, x2 - x1, y2 - y1);
      ctx.globalAlpha = 1;
    }

    // Playhead, drawn last so nothing paints over it.
    //
    // ⛔ Snapped to a whole device pixel (TASK-041F). A 2 px bar at a fractional
    // x is resampled across three columns at any DPR, which reads as a soft
    // grey smear rather than a marker — and it is the one thing on screen the
    // eye tracks continuously, so the blur is the most visible on the page.
    if (playhead > 0) {
      const x = crisp(GUTTER + tickToX(playhead * totalTicks, view), dpr);
      if (x >= GUTTER && x <= width) {
        ctx.fillStyle = palette.signal;
        ctx.fillRect(x, 0, Math.max(1, crisp(2, dpr)), height);
      }
    }
    ctx.restore();

    // Keyboard gutter, painted after the clip so it is never scrolled over.
    ctx.fillStyle = palette.surface2;
    ctx.fillRect(0, 0, GUTTER, height);
    // Font and baseline set once: the setter re-parses the shorthand and does a
    // font lookup on every assignment, and this used to run per labelled row.
    ctx.font = '10px system-ui, sans-serif';
    ctx.textBaseline = 'middle';
    rows.pitches.forEach((pitch, index) => {
      const y = rowY(index, view);
      ctx.fillStyle = isBlackKey(pitch) ? palette.surface : palette.text;
      ctx.fillRect(0, y + 1, GUTTER - 6, rowHeight - 2);
      // ⛔ Under a fold, *every* visible row is labelled. Only C is labelled
      // normally because the eye counts rows from the octave line — but a fold
      // removes rows from the middle, so counting stops working and an unlabelled
      // folded roll is unreadable.
      const label = folded ? true : pitch % 12 === 0;
      if (label && rowHeight >= 12) {
        ctx.fillStyle = palette.text3;
        ctx.fillText(pitchName(pitch), 4, y + rowHeight / 2);
      }
    });
    ctx.fillStyle = palette.border;
    ctx.fillRect(GUTTER - 1, 0, 1, height);
  }, [
    notes,
    selection,
    drag,
    view,
    snap,
    pattern,
    playhead,
    rowHeight,
    scrollTick,
    keepRow,
    classes,
    keyRoot,
    lane,
    folded,
    paletteOf,
  ]);

  // Redraw whenever anything the drawing reads changes. rAF-coalesced so a
  // burst of store writes in one tick costs one paint.
  useEffect(() => {
    const frame = window.requestAnimationFrame(draw);
    return () => window.cancelAnimationFrame(frame);
  }, [draw]);

  // ⛔ **The observer is created once and reaches `draw` through a ref.** With
  // `draw` in the dependency array this tore down and rebuilt a ResizeObserver
  // thirty times a second while the transport ran — `playhead` rebuilds `draw`,
  // and the observer has nothing to do with the playhead.
  // ⛔ **The observer is built once and asks for a *render*, not for a draw.**
  // Holding `draw` in a ref would mean mutating that ref from an effect, which
  // `react-hooks/immutability` forbids — and listing `draw` as a dependency was
  // the original bug: it rebuilt a ResizeObserver thirty times a second, because
  // the playhead rebuilds `draw` and the observer has nothing to do with the
  // playhead. `bump` from `useReducer` is stable for the component's lifetime,
  // so the effect below genuinely runs once.
  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => bump());
    observer.observe(wrap);
    return () => observer.disconnect();
  }, [bump]);

  // ---- pointer ----------------------------------------------------------

  /**
   * Where a pointer event falls, in the pattern's own units.
   *
   * ⛔ Takes a `MouseEvent`, which `PointerEvent` extends, so the double-click
   * and context-menu handlers use this one projection instead of each repeating
   * the rect-to-tick arithmetic. A hit test that rounds differently from the
   * draw is a note you can see and cannot click.
   *
   * `pitch` is `null` past the last drawn row: under a fold there is no pitch
   * there, and inventing one would place a note on a row nobody can see.
   */
  const locate = useCallback(
    (event: React.MouseEvent<HTMLCanvasElement>) => {
      const rect = event.currentTarget.getBoundingClientRect();
      const x = event.clientX - rect.left;
      const y = event.clientY - rect.top;
      const rows = rowsFor(view, rect.height, keepRow);
      return {
        x,
        tick: xToTick(x - GUTTER, view),
        pitch: yToPitch(y, rows, view),
        inGutter: x < GUTTER,
      };
    },
    [view, keepRow],
  );

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      if (event.button !== 0) return;
      const { x, tick, pitch, inGutter } = locate(event);
      // Below the last drawn row there is nothing to act on — under a fold that
      // is real space rather than an edge case.
      if (pitch === null) return;

      // The gutter auditions rather than edits — clicking a key is how a
      // producer finds the register they want before drawing in it.
      if (inGutter) {
        void auditionNote(pitch, part);
        return;
      }

      const hit = noteAt(pattern, lane, tick, pitch, minHitTicks);
      const editing = useEditing.getState();

      if (hit === null) {
        // Empty canvas: rubber-band. `Shift` keeps what was already selected,
        // which is the only way to build a selection across two regions.
        if (!event.shiftKey) editing.clearSelection();
        anchor.current = { tick, pitch };
        moved.current = false;
        editing.setDrag({
          kind: 'marquee',
          fromTick: tick,
          toTick: tick,
          fromPitch: pitch,
          toPitch: pitch,
        });
        event.currentTarget.setPointerCapture(event.pointerId);
        return;
      }

      const id = noteId(hit);
      if (event.shiftKey || event.ctrlKey || event.metaKey) {
        editing.toggleSelected(id);
      } else if (!editing.selection.has(id)) {
        // Clicking an unselected note selects just it.
        editing.select([id]);
      } else {
        // ⛔ **Already selected: hold the whole selection until pointerup.**
        // Collapsing here would break the multi-note drag — grabbing one of six
        // selected notes would drop the other five before the gesture started.
        // Collapsing *never* would strand a producer in a six-note selection
        // with no way to reduce it but clicking empty canvas first, which is not
        // what a DAW does. So the decision waits for the mouse to come back up
        // and is made on whether the pointer actually moved.
        collapseTo.current = id;
      }

      const noteX = GUTTER + tickToX(hit.startTick, view);
      const noteW = ticksToWidth(hit.lenTicks, view);
      const edge = edgeAt(x, noteX, noteW);

      // ⛔ The anchor holds only what the store cannot: where the gesture
      // *started*. It used to also carry `mode` and `edge`, which are exactly
      // what `drag.kind` and `drag.edge` already say one line below — two
      // records of one fact, and a dummy `edge: 'end'` written twice to satisfy
      // a field that a marquee has no opinion about.
      anchor.current = { tick, pitch };
      moved.current = false;
      editing.setDrag(
        edge === null
          ? { kind: 'move', deltaTicks: 0, deltaPitch: 0, copy: event.altKey }
          : { kind: 'resize', edge, deltaTicks: 0 },
      );
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [locate, pattern, lane, part, view, minHitTicks],
  );

  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const start = anchor.current;

      // ⛔ **The drag affordance appears on hover rather than living on screen**
      // (TASK-041F). A roll that drew a grip on every note at all times is a
      // roll with a hundred grips, and the notes stop being the thing the eye
      // finds first. The cursor is the affordance: `ew-resize` inside the ≥6 px
      // edge zone that `edgeAt` already owns, `move` over the body. One source
      // for both, so what the cursor promises is what the pointer does.
      if (start === null) {
        const { x, tick, pitch, inGutter } = locate(event);
        const hit =
          pitch === null || inGutter ? null : noteAt(pattern, lane, tick, pitch, minHitTicks);
        // ⛔⛔ **Three cursors, and Mike named all three on 2026-08-06:** *"i do
        // not want the '+' cursor for the piano rolls, only the '[' and ']'
        // cursors for resizing notes and a regular mousepointer"*, and then
        // *"can you also ensure that we have this for extending/shortening a
        // note for the left and right sides of all generators."*
        //
        // ⚠ **`w-resize` and `e-resize`, not one `ew-resize` for both.** They
        // are the bracket-shaped cursors, and which way the bracket faces says
        // which end of the note is about to move — the left edge changes where
        // it starts, the right edge changes how long it is. A single
        // double-headed arrow says "this resizes" and leaves the producer to
        // find out which end the hard way.
        const edge =
          hit === null
            ? null
            : edgeAt(
                x,
                GUTTER + tickToX(hit.startTick, view),
                ticksToWidth(hit.lenTicks, view),
              );
        const cursor =
          hit === null
            ? // ⛔ Was `crosshair`. A "+" over empty grid reads as "click here to
              // add", which is the draw-tool promise this roll does not make —
              // a plain click selects. See `PianoRoll.css` for the same change.
              'default'
            : edge === 'start'
              ? 'w-resize'
              : edge === 'end'
                ? 'e-resize'
                : 'move';
        if (event.currentTarget.style.cursor !== cursor) {
          event.currentTarget.style.cursor = cursor;
        }
        const id = hit === null ? null : noteId(hit);
        if (hovered.current !== id) {
          hovered.current = id;
          bump();
        }
        return;
      }
      const editing = useEditing.getState();
      const current = editing.drag;
      // The gesture's kind is the store's to say — see the anchor's note.
      if (current === null) return;

      const { tick, pitch } = locate(event);

      // ⛔ **Recorded from the pointer, before any clamping.** This is what
      // separates a click from a drag on pointerup, and the clamped delta
      // cannot: dragging left against tick 0 clamps to zero while the producer
      // is plainly dragging. See `moved`'s own note.
      if (tick !== start.tick || (pitch !== null && pitch !== start.pitch)) {
        moved.current = true;
      }

      if (current.kind === 'marquee') {
        editing.setDrag({
          kind: 'marquee',
          fromTick: start.tick,
          toTick: tick,
          // Past the last row the band simply stops growing downward rather
          // than snapping back to the start pitch.
          fromPitch: start.pitch,
          toPitch: pitch ?? current.toPitch,
        });
        return;
      }

      // ⛔ The delta is snapped, not the destination. Snapping the destination
      // makes a note that was already off-grid jump onto it the instant it is
      // touched — which silently destroys the humanized placement the engine
      // worked to produce, and is the single most complained-about behaviour in
      // editors that get this wrong.
      const rawDelta = tick - start.tick;
      const deltaTicks = snapTick(Math.abs(rawDelta), snap, pattern.ppq) * Math.sign(rawDelta);

      if (current.kind === 'resize') {
        editing.setDrag({ kind: 'resize', edge: current.edge, deltaTicks });
        return;
      }
      if (current.kind === 'move') {
        // ⛔ Clamped on the way *into* the store, so the preview the canvas
        // draws is the delta the commit will use — see .
        const held = clampedMove(
          pattern,
          lane,
          editing.selection,
          deltaTicks,
          pitch === null ? current.deltaPitch : pitch - start.pitch,
        );
        editing.setDrag({
          kind: 'move',
          deltaTicks: held.deltaTicks,
          // ⛔ The difference between the pitch *under the pointer* now and at
          // the start, which under a fold is the number of semitones the
          // visible rows actually span — dragging one row down in a folded roll
          // moves to the next row shown, not to the next semitone.
          deltaPitch: held.deltaPitch,
          copy: event.altKey,
        });
      }
    },
    [locate, snap, pattern, lane, view, bump, minHitTicks],
  );

  const onPointerUp = useCallback(() => {
    const start = anchor.current;
    anchor.current = null;
    const editing = useEditing.getState();
    const current = editing.drag;
    const collapse = collapseTo.current;
    collapseTo.current = null;
    editing.setDrag(null);
    if (start === null || current === null) return;

    // A click on an already-selected note that never became a drag: reduce the
    // selection to that one note, the way every DAW's arrange and roll do.
    //
    // ⛔ **Guarded on whether the *pointer* moved, not on the deltas.** The
    // deltas come out of `clampedMove`, so a real drag the clamp pins to zero —
    // dragging a selection left when its leftmost note already sits at tick 0,
    // or down against the bottom of the register — read as a click and silently
    // dropped the other five notes. The producer, still believing six were
    // held, pressed Delete and one responded.
    if (collapse !== null && current.kind === 'move' && !moved.current) {
      editing.select([collapse]);
      return;
    }

    if (current.kind === 'marquee') {
      const hit = notesInBox(pattern, lane, current);
      if (hit.length > 0) editing.addToSelection(hit);
      return;
    }

    // One commit, one undo step — see `editPattern`.
    if (current.kind === 'move') {
      // ⛔ **`copy` is finally read here** (TASK-041A). It was set from the
      // modifier on pointerdown and on every pointermove and then dropped on
      // the floor, so Alt-dragging — the duplicate gesture every DAW has —
      // *moved* the original: the note disappeared from where it had been.
      //
      // A copy is `duplicateNotes` at zero offset followed by the move, so the
      // originals stay put and the new notes take both the delta and the
      // selection. Still one `editPattern`, so still one undo step.
      const next = moveNotes(
        pattern,
        lane,
        editing.selection,
        current.deltaTicks,
        current.deltaPitch,
        current.copy,
      );
      editPattern(next);
      // The copies are what the producer is now holding, so they take the
      // selection — the same rule `Shift+D` follows.
      if (current.copy) editing.select(reselect(pattern, next, lane, editing.selection));
      return;
    }
    if (current.kind === 'resize') {
      const floor = stepTicks(snap, pattern.ppq);
      editPattern(
        resizeNotes(pattern, lane, editing.selection, current.edge, current.deltaTicks, floor),
      );
    }
  }, [pattern, lane, editPattern, snap]);

  /** Double-click on empty canvas draws a note, Ableton's own gesture. */
  const onDoubleClick = useCallback(
    (event: React.MouseEvent<HTMLCanvasElement>) => {
      const { tick, pitch, inGutter } = locate(event);
      if (inGutter || pitch === null) return;
      if (noteAt(pattern, lane, tick, pitch, minHitTicks) !== null) return;

      const step = stepTicks(snap, pattern.ppq);
      const placed: Note = {
        startTick: snapTick(tick, snap, pattern.ppq),
        lenTicks: step,
        pitch,
        // The engine's own mid tier rather than a flat 127: a note drawn at
        // full velocity beside generated ones stands out as an accent nobody
        // asked for.
        vel: 100,
      };
      // ⛔ The id comes from the *result*, because  clamps and a
      //  is the two fields it clamps. Double-clicking in the dimmed
      // space past the clip end drew a note that was then not selected.
      const next = addNote(pattern, lane, placed);
      editPattern(next);
      useEditing.getState().select(addedIds(pattern, next, lane));
      void auditionNote(pitch, part);
    },
    [locate, pattern, lane, snap, editPattern, part, minHitTicks],
  );

  /** Right-click deletes, which is FL Studio's gesture and costs no selection. */
  const onContextMenu = useCallback(
    (event: React.MouseEvent<HTMLCanvasElement>) => {
      const { tick, pitch, inGutter } = locate(event);
      if (inGutter || pitch === null) return;
      const hit = noteAt(pattern, lane, tick, pitch, minHitTicks);
      if (hit === null) return;
      event.preventDefault();
      editPattern(deleteNotes(pattern, lane, new Set<NoteId>([noteId(hit)])));
    },
    [locate, pattern, lane, editPattern, minHitTicks],
  );

  // ---- the velocity lane (TASK-041V) -------------------------------------

  // Both memoised because they are the lane's memo dependencies: a fresh array
  // or arrow function every render would rebuild every stem on every playhead
  // tick, which is the cost the canvas exists to avoid.
  const velocityTracks = useMemo(() => [{ lane, notes }], [lane, notes]);
  const velocityX = useCallback((tick: number) => tickToX(tick, view), [view]);

  const onPointerLeave = useCallback(() => {
    if (hovered.current === null) return;
    hovered.current = null;
    bump();
  }, [bump]);

  /** Ctrl/Cmd+wheel zooms; plain wheel scrolls. Both are the DAW convention. */
  const onWheel = useCallback(
    (event: React.WheelEvent<HTMLCanvasElement>) => {
      const editing = useEditing.getState();
      if (event.ctrlKey || event.metaKey) {
        editing.setZoom(editing.zoom * (event.deltaY < 0 ? 1.15 : 1 / 1.15));
        return;
      }
      if (event.shiftKey) {
        editing.scrollTo(
          editing.scrollTick + (event.deltaY / zoom) * pattern.ppq,
          editing.topPitch,
        );
        return;
      }
      editing.scrollTo(editing.scrollTick, editing.topPitch - Math.sign(event.deltaY) * 3);
    },
    [zoom, pattern.ppq],
  );

  return (
    <div className="roll">
      <RollBar scale={scale} pattern={pattern}>
        <TransformMenu pattern={pattern} lane={lane} classes={classes} />
      </RollBar>
      {/* The ruler shares the roll's tick mapping and gutter, so the brace and
          the grid below it cannot disagree about where a bar is. */}
      <Ruler pattern={pattern} lane={lane} view={view} gutter={GUTTER} />

      <div className="roll__canvasbox" ref={wrapRef}>
        {/*
        The view, published as data.

        A canvas has no geometry anything outside it can read, so without this a
        test that wants to click a specific note has to hardcode the defaults and
        silently starts clicking the wrong row the day one of them changes. These
        are the five numbers `geometry.ts` needs to turn a tick and a pitch into
        a pixel, and they are useful for exactly the same reason when debugging a
        misplaced click by hand.
      */}
        <canvas
          ref={canvasRef}
          className="roll__canvas"
          data-gutter={GUTTER}
          data-zoom={zoom}
          data-row-height={rowHeight}
          data-scroll-tick={scrollTick}
          data-top-pitch={topPitch}
          data-ppq={pattern.ppq}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          // Leaving takes the affordance with it, so a stale outline cannot sit
          // on a note nobody is pointing at.
          onPointerLeave={onPointerLeave}
          onDoubleClick={onDoubleClick}
          onContextMenu={onContextMenu}
          onWheel={onWheel}
        />
      </div>

      {/* The lane shares the roll's own tick mapping, so a cap is always under
          the note it belongs to at every zoom and scroll position. */}
      <VelocityLane
        pattern={pattern}
        tracks={velocityTracks}
        gutter={GUTTER}
        xOf={velocityX}
        selection={selection}
      />

      <NoteList notes={notes} selection={selection} />
    </div>
  );
}
