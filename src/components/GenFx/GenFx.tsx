import { useEffect, useRef, useState } from 'react';

import {
  accelerate,
  colourAt,
  CROSSFADE_MS,
  durationFor,
  ignition,
  MAX_DURATION_MS,
  sweepAt,
} from './ripple';
import { useUi } from '../../state/ui';
import './GenFx.css';

/**
 * The generation animation (FR-017, TASK-029).
 *
 * A violet→cyan ripple sweeps the grid, cells igniting as it passes and
 * settling behind it. It exists to mask generation latency: a beat that simply
 * appears reads as nothing having happened, and tens of milliseconds is too
 * short to feel like an answer to a click.
 *
 * **Nothing here re-renders React while it animates.** The frame loop writes to
 * a canvas through a ref and the run state lives in another; there is no
 * `setState` on the animating path at all. A state change per frame would
 * re-render the whole grid sixty times a second to draw an overlay that is not
 * part of it, which is how a 4 ms frame budget is missed.
 *
 * `prefers-reduced-motion` replaces all of it with a 150 ms crossfade, and the
 * canvas is never mounted at all. That is a different path rather than a
 * shorter animation, because the request is "no motion", not "less of it".
 */

/** Columns drawn. Independent of the pattern: this is a sweep, not a grid. */
const COLUMNS = 64;

/** What an empty column still gets, so a sparse pattern is not an invisible
 *  animation. */
const BASE_GLOW = 0.25;

/** Map a drawn column onto a density bucket, whatever length that array is. */
function columnOf(drawn: number, buckets: number): number {
  return Math.min(buckets - 1, Math.floor((drawn / COLUMNS) * buckets));
}

type Run = { startedAt: number; duration: number };

export function GenFx({
  active,
  density,
  children,
}: {
  active: boolean;
  /**
   * How busy each column of the finished pattern is, 0–1, or `undefined`
   * while there is nothing to light up.
   *
   * This is what makes cells ignite *as their own notes land* rather than a
   * uniform bar wiping across (FR-017). It arrives mid-animation — the whole
   * point of the ripple is that the pattern does not exist when it starts —
   * so the sweep begins even and the notes appear under it as they are made.
   */
  density?: number[];
  children: React.ReactNode;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const run = useRef<Run | null>(null);
  // Either source turns the animation off; neither can turn it back on over
  // the other. Someone who has asked for less motion, by whichever route, has
  // asked once (FR-017).
  const systemReduced = usePrefersReducedMotion();
  const settingReduced = useUi((s) => s.reduceMotion);
  const reduced = systemReduced || settingReduced;

  // Read by the frame loop, which must not re-subscribe when it changes.
  const densityRef = useRef<number[] | undefined>(density);
  useEffect(() => {
    densityRef.current = density;
  }, [density]);

  useEffect(() => {
    if (reduced) return;

    if (active) {
      // The run opens at the ceiling, because how long generation will take is
      // not knowable yet.
      run.current = { startedAt: performance.now(), duration: MAX_DURATION_MS };
    } else if (run.current) {
      // It landed. Scale the remaining sweep rather than cutting it off, so it
      // accelerates into the landing instead of snapping to it (FR-017) — and
      // never below the floor, since an animation under 300 ms is a flicker
      // rather than an answer to a click.
      const elapsed = performance.now() - run.current.startedAt;
      run.current.duration = Math.max(
        durationFor(elapsed),
        accelerate(elapsed, run.current.duration),
      );
    }

    const state = run.current;
    const canvas = canvasRef.current;
    if (!state || !canvas) return;

    const context = canvas.getContext('2d');
    if (!context) return; // No 2D context: the pattern simply appears.

    let frame = 0;
    const draw = () => {
      // Match the backing store to the box — the grid resizes with the window,
      // and a stale size draws the ripple stretched.
      const { width, height } = canvas.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const pixelWidth = Math.max(1, Math.round(width * dpr));
      const pixelHeight = Math.max(1, Math.round(height * dpr));
      if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
      if (canvas.height !== pixelHeight) canvas.height = pixelHeight;

      const elapsed = performance.now() - state.startedAt;
      const sweep = sweepAt(elapsed, state.duration);
      context.clearRect(0, 0, canvas.width, canvas.height);

      const columnWidth = canvas.width / COLUMNS;
      const notes = densityRef.current;
      for (let i = 0; i < COLUMNS; i += 1) {
        const x = i / COLUMNS;
        const lit = ignition(x, sweep, state.duration);
        if (lit <= 0) continue;

        // Where the notes are, the column ignites; where they are not, the
        // sweep still shows but faintly. Without the floor the ripple would
        // disappear entirely over a sparse pattern, which reads as the
        // animation being broken rather than as the beat being sparse.
        const weight = notes
          ? BASE_GLOW + (1 - BASE_GLOW) * (notes[columnOf(i, notes.length)] ?? 0)
          : 1;

        const { r, g, b } = colourAt(x);
        context.fillStyle = `rgba(${r}, ${g}, ${b}, ${lit * weight * 0.55})`;
        // +1 so neighbouring columns meet rather than leaving hairlines.
        context.fillRect(i * columnWidth, 0, columnWidth + 1, canvas.height);
      }

      if (elapsed >= state.duration) {
        context.clearRect(0, 0, canvas.width, canvas.height);
        run.current = null;
        hostRef.current?.removeAttribute('data-running');
        return;
      }
      frame = requestAnimationFrame(draw);
    };

    hostRef.current?.setAttribute('data-running', 'true');
    frame = requestAnimationFrame(draw);
    // `active` flipping mid-run tears this down and immediately sets it up
    // again with the compressed duration. The sweep does not jump: progress is
    // measured from `startedAt`, not counted in frames.
    return () => cancelAnimationFrame(frame);
  }, [active, reduced]);

  // The reduced-motion path: a class for the length of the crossfade, and no
  // canvas at any point.
  useEffect(() => {
    if (!reduced || !active) return;
    const host = hostRef.current;
    host?.setAttribute('data-running', 'true');
    const timer = window.setTimeout(() => host?.removeAttribute('data-running'), CROSSFADE_MS);
    return () => window.clearTimeout(timer);
  }, [active, reduced]);

  return (
    <div
      ref={hostRef}
      className={`genfx genfx--${reduced ? 'crossfade' : 'ripple'}`}
      data-testid="genfx"
    >
      {children}
      {/* Aria-hidden and pointer-events: none — decoration over the real grid,
          which must never intercept a click or be announced. */}
      {!reduced && <canvas ref={canvasRef} className="genfx__canvas" aria-hidden />}
    </div>
  );
}

/**
 * The OS's reduced-motion setting, kept live.
 *
 * Subscribed to rather than read once: someone who turns it on mid-session
 * because the animation is making them ill should not have to restart the app
 * to be listened to.
 */
function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(
    () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false,
  );

  useEffect(() => {
    const query = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    if (!query) return;
    const onChange = (event: MediaQueryListEvent) => setReduced(event.matches);
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  return reduced;
}
