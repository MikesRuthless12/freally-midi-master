import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useEditing } from '../../state/editing';
import { useSession } from '../../state/session';
import { useCanvasPalette } from './palette';
import type { Pattern } from '../../lib/ipc-types';
import { mapNotes, noteId, type NoteId } from './notes';
import {
  CAP_RADIUS,
  GRAB_PX,
  LANE_HEIGHT,
  STEM_WIDTH,
  line,
  reset,
  shift,
  stemNear,
  stemsFor,
  sweep,
  velocityToY,
  yToVelocity,
  type Stem,
  type Track,
  type Values,
} from './velocity';
import './VelocityLane.css';

/**
 * The velocity lane, drawn the way FL Studio and Ableton draw it (TASK-041V).
 *
 * One skinny stem per note with a round cap at the top: the stem is the value,
 * the cap is the handle. Dragging sideways paints every slider the pointer
 * passes at the pointer's own height — which is what makes sixteen hats one
 * gesture instead of sixteen, and is the reason the lane is usable at all.
 *
 * ⛔ **Canvas, and it belongs to whatever editor mounts it.** The host supplies
 * the tick→x mapping so the caps line up with the notes above them, whether
 * those are drawn by the roll's zoomable view or by the drum grid's fixed
 * sixteen columns. A lane with its own opinion about where a tick sits would
 * drift from the grid it is supposed to be under, and the producer would be
 * dragging the wrong note's velocity while watching the right one.
 */

/** What a drag in the lane is doing. */
type Gesture = {
  /** `paint` writes the pointer's height; `relative` moves a selection as one. */
  mode: 'paint' | 'relative' | 'reset';
  /** Where the drag began, for `Shift`'s straight line and the relative delta. */
  anchor: { x: number; vel: number };
  /** The last position, so a sweep only covers the segment just travelled. */
  prevX: number;
  /** Everything the gesture has written so far. */
  values: Values;
  /** Which pointer owns the gesture, so a second one cannot join it. */
  pointerId: number;
};

/**
 * The lane's text alternative, and where a test reads what the caps say.
 *
 * The same bargain the roll's note list makes: a canvas is invisible to both a
 * screen reader and Playwright, so what it draws is published as data next to
 * it. `data-x` is what a pointer-driven test needs to find a cap without
 * re-deriving the layout and silently drifting from it.
 */
const StemList = memo(function StemList({
  stems,
  label,
}: {
  stems: readonly Stem[];
  label: string;
}) {
  const { t } = useTranslation();
  return (
    <ul className="visually-hidden" data-testid="velocity-stems" aria-label={label}>
      {stems.map((stem) => (
        <li
          key={stem.id}
          data-stem={stem.id}
          data-lane={stem.lane}
          data-tick={stem.note.startTick}
          data-pitch={stem.note.pitch}
          data-vel={stem.note.vel}
          data-model-vel={stem.note.modelVel ?? undefined}
          data-x={Math.round(stem.x)}
        >
          {t('roll.velocityOf', { tick: stem.note.startTick, velocity: stem.note.vel })}
        </li>
      ))}
    </ul>
  );
});

export function VelocityLane({
  pattern,
  tracks,
  gutter,
  xOf,
  selection,
}: {
  pattern: Pattern;
  /** The lanes whose notes the lane shows. The roll passes one; the kit passes all. */
  tracks: readonly Track[];
  /** How much of the canvas is the editor's own gutter, before the timeline. */
  gutter: number;
  /** Tick → x inside the timeline area, given how wide it currently is. */
  xOf: (tick: number, width: number) => number;
  /**
   * The host's selection, as unqualified note ids.
   *
   * Only the roll has one, and it edits a single lane — so an unqualified id is
   * unambiguous there. The kit passes nothing rather than a set that could match
   * a note in a lane the producer never selected.
   */
  selection?: ReadonlySet<NoteId>;
}) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const paletteOf = useCanvasPalette(wrapRef);
  const editPattern = useSession((s) => s.editPattern);
  const drag = useEditing((s) => s.drag);
  const gesture = useRef<Gesture | null>(null);

  /**
   * The timeline's width, in state rather than read at draw time.
   *
   * ⛔ **The layout is an input to the hit test, not just to the paint.** The
   * drum grid's mapping is "the clip across the available width", so every cap's
   * x moves when the panel resizes — and a width known only inside the draw
   * would leave `stemNear` testing against last month's layout. Both come from
   * this one number instead.
   */
  const [width, setWidth] = useState(0);

  const stems = useMemo(
    () => stemsFor(tracks, (tick) => xOf(tick, Math.max(0, width - gutter))),
    [tracks, xOf, gutter, width],
  );

  /** The value a stem is showing right now, preview included. */
  const shownVel = useCallback(
    (stem: Stem) => {
      if (drag?.kind !== 'velocity') return stem.note.vel;
      return drag.values[stem.id] ?? stem.note.vel;
    },
    [drag],
  );

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    const ctx = canvas.getContext('2d');
    if (ctx === null) return;

    const height = wrap.clientHeight;
    if (width === 0 || height === 0) return;

    // DPR-aware and re-read every draw, for the reason the roll's canvas is:
    // the window can move between monitors while it is open.
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

    ctx.fillStyle = palette.surface2;
    ctx.fillRect(0, 0, width, height);

    // The baseline the stems rise from, and the gutter's edge, so the lane
    // reads as part of the editor above rather than a separate strip.
    ctx.fillStyle = palette.border;
    ctx.fillRect(gutter, height - 1, width - gutter, 1);
    if (gutter > 0) ctx.fillRect(gutter - 1, 0, 1, height);

    const baseline = height - 1;
    for (const stem of stems) {
      const x = gutter + stem.x;
      if (x < gutter - CAP_RADIUS || x > width + CAP_RADIUS) continue;

      const vel = shownVel(stem);
      const y = velocityToY(vel, height);
      const selected = selection !== undefined && selection.has(noteId(stem.note));
      // The same two colours the roll paints its notes in, so the lane and the
      // grid can never disagree about what is selected.
      ctx.fillStyle = selected ? palette.signal : palette.primary;

      ctx.globalAlpha = 0.55;
      ctx.fillRect(x - STEM_WIDTH / 2, y, STEM_WIDTH, baseline - y);
      ctx.globalAlpha = 1;

      ctx.beginPath();
      ctx.arc(x, y, CAP_RADIUS, 0, Math.PI * 2);
      ctx.fill();
    }
  }, [stems, gutter, selection, shownVel, width, paletteOf]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(draw);
    return () => window.cancelAnimationFrame(frame);
  }, [draw]);

  // A resize changes every x, and nothing in the store moves when it happens.
  useEffect(() => {
    const wrap = wrapRef.current;
    if (wrap === null) return;
    setWidth(wrap.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => setWidth(wrap.clientWidth));
    observer.observe(wrap);
    return () => observer.disconnect();
  }, []);

  /** Where a pointer event lands, in the lane's own coordinates. */
  const locate = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const box = event.currentTarget.getBoundingClientRect();
      return {
        x: event.clientX - box.left - gutter,
        vel: yToVelocity(event.clientY - box.top, box.height),
      };
    },
    [gutter],
  );

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      // Only the two buttons that mean something here. Middle-click autoscroll
      // through a paint gesture would write velocities nobody asked for.
      if (event.button !== 0 && event.button !== 2) return;
      const { x, vel } = locate(event);
      const hit = stemNear(stems, x);

      // ⛔ **A drag that starts on a *selected* cap moves the whole selection,
      // relatively.** Painting it instead would flatten the accent pattern the
      // producer selected precisely because they wanted to keep its shape.
      const onSelected =
        hit !== null && selection !== undefined && selection.has(noteId(hit.note));
      const mode: Gesture['mode'] =
        event.button === 2 ? 'reset' : onSelected && selection.size > 1 ? 'relative' : 'paint';

      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();

      const start: Gesture = {
        mode,
        anchor: { x, vel },
        prevX: x,
        values: {},
        pointerId: event.pointerId,
      };
      gesture.current = start;

      // The gesture writes on the way down as well as on the way across: a
      // single click on a cap is a legitimate way to set one value.
      start.values =
        mode === 'reset'
          ? reset(stems, new Set(Object.keys(sweep(stems, x, x, vel))))
          : mode === 'relative'
            ? {}
            : sweep(stems, x, x, vel);
      useEditing.getState().setDrag({ kind: 'velocity', values: start.values });
    },
    [locate, stems, selection],
  );

  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const current = gesture.current;
      if (current === null || current.pointerId !== event.pointerId) return;
      const { x, vel } = locate(event);

      if (current.mode === 'relative') {
        // ⛔ Alt scrubs finely — a quarter of the travel, so an accent can be
        // trimmed by a few units without the pointer leaving the lane.
        const scale = event.altKey ? 0.25 : 1;
        const ids = new Set(
          stems
            .filter((stem) => selection !== undefined && selection.has(noteId(stem.note)))
            .map((stem) => stem.id),
        );
        current.values = shift(stems, ids, (vel - current.anchor.vel) * scale);
      } else if (current.mode === 'reset') {
        const swept = sweep(stems, current.prevX, x, vel);
        Object.assign(current.values, reset(stems, new Set(Object.keys(swept))));
      } else if (event.shiftKey) {
        // The straight line replaces the gesture rather than adding to it: it is
        // defined by its two ends, so dragging back has to redraw the ramp.
        current.values = line(stems, current.anchor, { x, vel });
      } else {
        Object.assign(current.values, sweep(stems, current.prevX, x, vel));
      }

      current.prevX = x;
      useEditing.getState().setDrag({ kind: 'velocity', values: { ...current.values } });
    },
    [locate, stems, selection],
  );

  const commit = useCallback(
    (values: Values) => {
      // One commit for the whole gesture, so a paint across sixteen hats is one
      // undo step rather than sixteen — `editPattern` snapshots per call.
      const byLane = new Map<Stem['lane'], Map<NoteId, number>>();
      for (const stem of stems) {
        const vel = values[stem.id];
        if (vel === undefined || vel === stem.note.vel) continue;
        const at = byLane.get(stem.lane) ?? new Map<NoteId, number>();
        at.set(noteId(stem.note), vel);
        byLane.set(stem.lane, at);
      }
      if (byLane.size === 0) return;

      let next = pattern;
      for (const [lane, wanted] of byLane) {
        next = mapNotes(next, lane, new Set(wanted.keys()), (note) => ({
          ...note,
          vel: wanted.get(noteId(note)) ?? note.vel,
        }));
      }
      editPattern(next);
    },
    [stems, pattern, editPattern],
  );

  const onPointerUp = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const current = gesture.current;
      if (current === null || current.pointerId !== event.pointerId) return;
      gesture.current = null;
      useEditing.getState().setDrag(null);
      commit(current.values);
    },
    [commit],
  );

  /**
   * Double-click resets, for anyone who reaches for that before right-click.
   *
   * With notes selected it resets the whole selection, which is the one case
   * where the pointer's own position is not what the producer means.
   */
  const onDoubleClick = useCallback(
    (event: React.MouseEvent<HTMLCanvasElement>) => {
      const box = event.currentTarget.getBoundingClientRect();
      const x = event.clientX - box.left - gutter;
      const ids =
        selection !== undefined && selection.size > 0
          ? new Set(stems.filter((s) => selection.has(noteId(s.note))).map((s) => s.id))
          : new Set([stemNear(stems, x)?.id].filter((id): id is string => id !== undefined));
      if (ids.size === 0) return;
      commit(reset(stems, ids));
    },
    [gutter, stems, selection, commit],
  );

  return (
    <div className="velocity" ref={wrapRef} style={{ height: LANE_HEIGHT }}>
      <canvas
        ref={canvasRef}
        className="velocity__canvas"
        data-testid="velocity-lane"
        data-gutter={gutter}
        data-height={LANE_HEIGHT}
        data-grab={GRAB_PX}
        aria-label={t('roll.velocityLabel')}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={onDoubleClick}
        // The right button is the reset gesture; the menu would swallow it.
        onContextMenu={(event) => event.preventDefault()}
      />
      <StemList stems={stems} label={t('roll.velocityLabel')} />
    </div>
  );
}
