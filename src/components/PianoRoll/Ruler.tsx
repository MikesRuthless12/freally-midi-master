import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useEditing } from '../../state/editing';
import { useSession } from '../../state/session';
import { useCanvasPalette } from './palette';
import type { Lane, Pattern, Region } from '../../lib/ipc-types';
import { barTicks, noteId, notesOf, patternTicks, snapTick, stepTicks } from './notes';
import { tickToX, xToTick, type View } from './geometry';
import { reselect, stretch } from './transforms';
import {
  RULER_HEIGHT,
  STRETCH_BAND_TOP,
  clipOf,
  gripAt,
  loopOf,
  moveEdge,
  regionBetween,
  stretchFactor,
  type Grip,
} from './regions';
import './Ruler.css';

/**
 * The bar/beat ruler, the loop brace, the clip markers and the stretch handles
 * (TASK-041E, and 041D's handles).
 *
 * ⛔ **Its own canvas above the roll's, rather than a strip inside it.** The
 * roll's vertical arithmetic — `rowY`, `yToPitch`, the fold's row list — is
 * measured from the top of its canvas, and reserving a band inside it would put
 * an offset into every one of those conversions and into every test that
 * mirrors them. A sibling canvas sharing the same `tickToX` and gutter lines up
 * exactly and touches none of it.
 *
 * ⛔ **The regions live on the pattern**, so a brace survives a reload, travels
 * with a preset and steps back with undo — the same reasons the notes do.
 */

/** A drag in the strip, and what it started from. */
type Drag = {
  grip: Grip;
  pointerId: number;
  /** Where an empty-ruler drag began, in ticks. */
  fromTick: number;
  /** The selection's span when a stretch began — the factor is measured against it. */
  span: Region | null;
  /** What the drag would commit if it ended now. */
  pending: Partial<Pattern> | null;
};

export function Ruler({
  pattern,
  lane,
  view,
  gutter,
}: {
  pattern: Pattern;
  lane: Lane;
  view: View;
  gutter: number;
}) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const paletteOf = useCanvasPalette(wrapRef);
  const editPattern = useSession((s) => s.editPattern);
  const selection = useEditing((s) => s.selection);
  const snap = useEditing((s) => s.snap);
  const drag = useRef<Drag | null>(null);
  const [width, setWidth] = useState(0);
  /** The in-flight drag, mirrored into state so the strip repaints as it moves. */
  const [live, setLive] = useState<Partial<Pattern> | null>(null);

  const shown = live === null ? pattern : { ...pattern, ...live };
  const total = patternTicks(pattern);
  const loop = loopOf(shown);
  const region = clipOf(shown);

  /** The selection's outer edges, or `null` when there is nothing to stretch. */
  const span: Region | null = (() => {
    if (selection.size < 2) return null;
    const picked = notesOf(pattern, lane).filter((note) => selection.has(noteId(note)));
    if (picked.length < 2) return null;
    return {
      fromTick: Math.min(...picked.map((note) => note.startTick)),
      toTick: Math.max(...picked.map((note) => note.startTick + note.lenTicks)),
    };
  })();

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap || width === 0) return;
    const ctx = canvas.getContext('2d');
    if (ctx === null) return;

    const height = RULER_HEIGHT;
    const dpr = window.devicePixelRatio || 1;
    if (
      canvas.width !== Math.round(width * dpr) ||
      canvas.height !== Math.round(height * dpr)
    ) {
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const palette = paletteOf();
    if (palette === null) return;
    const at = (tick: number) => gutter + tickToX(tick, view);

    ctx.fillStyle = palette.surface2;
    ctx.fillRect(0, 0, width, height);

    // Bars and beats, drawn from the pattern's *own* meter — a 6/8 clip has
    // three-quarter bars, and a ruler that assumed four would number them wrong
    // from bar 2 onwards.
    const bar = barTicks(pattern);
    const beat = Math.max(1, bar / Math.max(1, pattern.timeSigNum));
    ctx.font = '10px system-ui, sans-serif';
    ctx.textBaseline = 'top';
    for (let tick = 0; tick <= total; tick += beat) {
      const x = at(tick);
      if (x < gutter || x > width) continue;
      const isBar = tick % bar === 0;
      ctx.fillStyle = isBar ? palette.text3 : palette.border;
      ctx.fillRect(x, isBar ? 0 : height - 6, 1, isBar ? height : 6);
      if (isBar) {
        ctx.fillStyle = palette.text3;
        ctx.fillText(String(Math.round(tick / bar) + 1), x + 3, 1);
      }
    }

    // The clip's own start and end, drawn under the brace: they are a boundary,
    // not a control the eye should go to first.
    ctx.fillStyle = palette.text3;
    for (const tick of [region.fromTick, region.toTick]) {
      const x = at(tick);
      if (x >= gutter && x <= width) ctx.fillRect(x - 1, height - 4, 3, 4);
    }

    // The loop brace.
    const from = Math.max(gutter, at(loop.fromTick));
    const to = Math.min(width, at(loop.toTick));
    if (to > from) {
      ctx.fillStyle = palette.primary;
      ctx.globalAlpha = 0.2;
      ctx.fillRect(from, 0, to - from, height);
      ctx.globalAlpha = 1;
      ctx.fillRect(from, 0, 2, height);
      ctx.fillRect(to - 2, 0, 2, height);
    }

    // The stretch handles, in the lower band and only when there is a selection
    // wide enough to stretch.
    if (span !== null) {
      ctx.fillStyle = palette.signal;
      for (const tick of [span.fromTick, span.toTick]) {
        const x = at(tick);
        if (x >= gutter && x <= width) {
          ctx.fillRect(x - 2, STRETCH_BAND_TOP, 4, height - STRETCH_BAND_TOP);
        }
      }
    }

    ctx.fillStyle = palette.surface2;
    ctx.fillRect(0, 0, gutter, height);
    ctx.fillStyle = palette.border;
    ctx.fillRect(gutter - 1, 0, 1, height);
    ctx.fillRect(0, height - 1, width, 1);
  }, [width, gutter, view, pattern, total, loop, region, span, paletteOf]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(draw);
    return () => window.cancelAnimationFrame(frame);
  }, [draw]);

  useEffect(() => {
    const wrap = wrapRef.current;
    if (wrap === null) return;
    setWidth(wrap.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => setWidth(wrap.clientWidth));
    observer.observe(wrap);
    return () => observer.disconnect();
  }, []);

  const locate = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const box = event.currentTarget.getBoundingClientRect();
      const x = event.clientX - box.left;
      return {
        x,
        y: event.clientY - box.top,
        tick: Math.min(total, Math.max(0, xToTick(x - gutter, view))),
      };
    },
    [gutter, view, total],
  );

  const step = stepTicks(snap, pattern.ppq);

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      if (event.button !== 0) return;
      const { x, y, tick } = locate(event);
      if (x < gutter) return;
      const grip = gripAt(x, y, loop, region, span, (t) => gutter + tickToX(t, view));
      event.currentTarget.setPointerCapture(event.pointerId);
      drag.current = { grip, pointerId: event.pointerId, fromTick: tick, span, pending: null };
    },
    [locate, gutter, loop, region, span, view],
  );

  /**
   * Where the drag has got to, held here rather than written to the pattern.
   *
   * ⛔ **A gesture is one undo step.** `editPattern` snapshots on every call and
   * `history.ts` lists `pattern` as discrete, so writing each pointermove
   * through would cost one Ctrl+Z per *frame* of a brace drag — the producer
   * would press undo eight times to take back one drag and reasonably conclude
   * undo was broken. The live note drag lives outside the pattern for exactly
   * this reason; so does this one.
   */
  const preview = useCallback(
    (current: Drag, tick: number): Partial<Pattern> | null => {
      const at = snapTick(tick, snap, pattern.ppq);
      switch (current.grip.kind) {
        case 'loop':
          return { loopRegion: moveEdge(loop, current.grip.edge, at, step, total) };
        case 'clip':
          return { clipRegion: moveEdge(region, current.grip.edge, at, step, total) };
        case 'new':
          return { loopRegion: regionBetween(current.fromTick, at, step, total) };
        // A stretch rewrites notes rather than a region, and is applied whole
        // on release by `onPointerUp`.
        case 'stretch':
          return null;
      }
    },
    [snap, pattern.ppq, loop, region, step, total],
  );

  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const current = drag.current;
      if (current === null || current.pointerId !== event.pointerId) return;
      const { tick } = locate(event);
      current.pending = preview(current, tick);
      setLive(current.pending);
    },
    [locate, preview],
  );

  const onPointerUp = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const current = drag.current;
      if (current === null || current.pointerId !== event.pointerId) return;
      drag.current = null;
      setLive(null);

      if (current.grip.kind === 'stretch') {
        if (current.span === null) return;
        const { tick } = locate(event);
        const factor = stretchFactor(
          current.span,
          current.grip.edge,
          snapTick(tick, snap, pattern.ppq),
        );
        if (factor === null || factor === 1) return;
        const next = stretch(pattern, lane, selection, factor);
        if (next === pattern) return;
        editPattern(next);
        useEditing.getState().select(reselect(pattern, next, lane, selection));
        return;
      }

      // One commit for the whole drag, so the brace is one step on the stack.
      if (current.pending !== null) editPattern({ ...pattern, ...current.pending });
    },
    [locate, snap, pattern, lane, selection, editPattern],
  );

  /** Double-click clears the loop, which is how every DAW takes a brace off. */
  const onDoubleClick = useCallback(() => {
    if (pattern.loopRegion === undefined || pattern.loopRegion === null) return;
    editPattern({ ...pattern, loopRegion: null });
  }, [pattern, editPattern]);

  return (
    <div className="ruler" ref={wrapRef} style={{ height: RULER_HEIGHT }}>
      <canvas
        ref={canvasRef}
        className="ruler__canvas"
        data-testid="roll-ruler"
        data-gutter={gutter}
        data-height={RULER_HEIGHT}
        data-loop-from={loop.fromTick}
        data-loop-to={loop.toTick}
        data-clip-from={region.fromTick}
        data-clip-to={region.toTick}
        aria-label={t('roll.rulerLabel')}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={onDoubleClick}
      />
    </div>
  );
}
