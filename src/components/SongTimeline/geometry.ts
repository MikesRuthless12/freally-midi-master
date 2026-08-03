/**
 * Where a bar lands on screen, and how fine the grid is at this zoom
 * (TASK-063B).
 *
 * Pure, and separate from the song data and the renderer, for the same reason
 * `PianoRoll/geometry.ts` is: the draw and the hit test must agree exactly, and
 * the only way to guarantee that is for both to call these. A clip you can see
 * and cannot grab is the classic timeline bug and it is invisible to any test
 * that only checks a handler fired.
 */

/** What the arrangement is looking at. Everything below derives from this. */
export type View = {
  /** Pixels per bar. */
  zoom: number;
  /** Leftmost visible bar. */
  scrollBar: number;
};

/** The zoom range, in pixels per bar. */
export const MIN_ZOOM = 6;
export const MAX_ZOOM = 240;

/**
 * One step of the zoom control, as a multiplier.
 *
 * Geometric rather than additive: at 8 px/bar a step of +8 doubles the scale
 * and at 200 it is invisible, so a linear step makes the control useless at one
 * end or the other.
 */
export const ZOOM_STEP = 1.5;

export function zoomIn(zoom: number): number {
  return Math.min(MAX_ZOOM, zoom * ZOOM_STEP);
}

export function zoomOut(zoom: number): number {
  return Math.max(MIN_ZOOM, zoom / ZOOM_STEP);
}

/** A bar's pixel offset from the left edge. */
export function barToX(bar: number, view: View): number {
  return (bar - view.scrollBar) * view.zoom;
}

/** Which bar a pixel offset falls on, as a fraction. */
export function xToBar(x: number, view: View): number {
  return x / view.zoom + view.scrollBar;
}

/**
 * How the grid is drawn at this zoom (TASK-063B).
 *
 * ⛔ **The resolution follows the zoom; it does not stay fixed.** That is the
 * whole requirement, and it is what Ableton and FL both do: zoomed out you see
 * bar lines and phrase lines, and as you zoom in beats appear and then
 * subdivisions. A fixed grid is either an unreadable smear of lines when zoomed
 * out or a featureless field when zoomed in.
 *
 * The thresholds are spacing-based rather than zoom-based so the rule reads as
 * what it is: a line is only drawn when there is room to see it. `beatStep` of
 * 0 means beats are not drawn at all.
 */
export type Grid = {
  /** Bars between the heavy lines. */
  barStep: number;
  /** Beats between the light lines; 0 when there is no room for any. */
  beatStep: number;
  /** Bars between ruler labels. */
  labelStep: number;
};

/** Fewer pixels than this between two lines and they read as one. */
const MIN_LINE_GAP = 7;
/** A ruler label needs room for its text. */
const MIN_LABEL_GAP = 44;

export function gridFor(view: View, beatsPerBar: number): Grid {
  const beats = Math.max(1, beatsPerBar);

  // Bar lines thin out in musical steps — 1, 2, 4, 8, 16 — because a producer
  // counts in phrases. Stepping by 3 or 5 would put the heavy lines somewhere
  // no section boundary ever falls.
  let barStep = 1;
  while (view.zoom * barStep < MIN_LINE_GAP) barStep *= 2;

  // Beats appear only once a whole bar is wide enough for them, and
  // subdivisions are deliberately not drawn: at this scale a clip is the unit
  // being moved, and 16ths would be noise.
  const beatWidth = view.zoom / beats;
  const beatStep = barStep === 1 && beatWidth >= MIN_LINE_GAP ? 1 : 0;

  let labelStep = barStep;
  while (view.zoom * labelStep < MIN_LABEL_GAP) labelStep *= 2;

  return { barStep, beatStep, labelStep };
}

/**
 * Where a bar sits in the song, as `bar|beat` — what the ruler prints.
 *
 * One-based, because every DAW counts bars from 1 and a producer reading "0"
 * assumes the display is broken.
 */
export function barLabel(bar: number): string {
  return String(bar + 1);
}

/**
 * The wall-clock time a bar starts at, in seconds.
 *
 * The ruler carries timestamps as well as bars (TASK-063B), because "is the
 * chorus inside 60 seconds" is a question about pop arrangements the research
 * states outright and bars alone cannot answer.
 */
export function barToSeconds(bar: number, bpm: number, beatsPerBar: number): number {
  const safeBpm = bpm > 0 && Number.isFinite(bpm) ? bpm : 120;
  return (bar * Math.max(1, beatsPerBar) * 60) / safeBpm;
}

/** `m:ss`, which is how a producer reads a timeline. */
export function formatTime(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(whole / 60);
  return `${minutes}:${String(whole % 60).padStart(2, '0')}`;
}
