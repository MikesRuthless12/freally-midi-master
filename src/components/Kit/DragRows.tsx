import { useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import { drawDragPreview, drawSongPreview } from './dragPreview';
import { LANE_ORDER } from '../DrumGrid/cells';
import {
  useDrag,
  THRESHOLD_PX,
  type DragFormat,
  type DragSubject,
  type Gesture,
} from '../../state/drag';
import { useSession } from '../../state/session';
import { useSong } from '../../state/song';
import { useStems } from '../../state/stems';
import type { Pattern } from '../../lib/ipc-types';

/**
 * Picking a generated part up and dropping it on a DAW track (TASK-063C).
 *
 * ⛔ **Not `draggable`, and not one `dragstart` handler anywhere.** An HTML5 drag
 * inside a webview is handed to the page rather than to the window server, so
 * the DAW never sees a file — the real drag is `DoDragDrop`, started by
 * `plugin/src/drag`. `state/drag.ts` carries the full note; what follows is the
 * pointer half of the same gesture.
 *
 * ⛔ **In the right rail, like everything else near the pattern.**
 * `stage__controls` sits under `stage__body`, so a row of chips there costs the
 * velocity lane its height and fails `e2e/piano-roll.spec.ts:380` — see
 * `StemsPanel` for the three attempts that proved it.
 */
export function DragRows() {
  const { t } = useTranslation();
  const canDrag = useDrag((s) => s.canDrag);
  const patterns = useSession((s) => s.patterns);
  const song = useSong((s) => s.song);
  const splitLanes = useStems((s) => s.splitLanes);
  const state = useDrag((s) => s.state);
  const message = useDrag((s) => s.message);

  // ⚠ Memoised because it rebuilds a `subject` object per row, and this
  // component re-renders on every step of every drag: without it no chip's
  // props are ever stable.
  const rows = useMemo(() => {
    const built: { key: string; label: string; subject: DragSubject; audio: boolean }[] = [];
    if (song) {
      // ⚠ **MIDI only for an arrangement**, and the plugin refuses the audio
      // half rather than freezing: a song is minutes long, and rendering one
      // needs progress a producer can watch and a cancel they can press.
      built.push({
        key: 'song',
        label: t('tabs.song'),
        subject: { kind: 'song', song },
        audio: false,
      });
    }

    for (const pattern of Object.values(patterns) as Pattern[]) {
      // "i can just drag the hihats out to the daw or just the snares" — one row
      // per lane. ⛔ The pattern goes over whole and the *lane is named*; the
      // plugin cuts it, because `export::Cut` is where the rule that a lane stem
      // is named for its lane already lives.
      if (splitLanes && pattern.lanes.length > 1) {
        // ⛔ `LANE_ORDER`, so these read top-to-bottom in the same order the
        // drum grid draws them. The engine's own lane order is whatever it
        // emitted, which is not the order anything else in the UI uses.
        for (const lane of LANE_ORDER) {
          const track = pattern.lanes.find((one) => one.lane === lane);
          if (!track || track.notes.length === 0) continue;
          built.push({
            key: `${pattern.part}-${lane}`,
            label: t(`lanes.${lane}`),
            subject: { kind: 'patterns', patterns: [pattern], lane },
            audio: true,
          });
        }
        continue;
      }
      built.push({
        key: pattern.part,
        label: t(`tabs.${pattern.part}`),
        subject: { kind: 'patterns', patterns: [pattern] },
        audio: true,
      });
    }
    return built;
  }, [patterns, song, splitLanes, t]);

  // ⛔ **Nothing is rendered where there is no drag source.** macOS and Linux
  // keep the Export buttons above and say "Export", deliberately — a handle that
  // drops nothing is the readout-that-lies failure this project has now written
  // down five times.
  if (!canDrag || rows.length === 0) return null;

  return (
    <div className="drag">
      <p className="kit-hint">{t('stems.dragHint')}</p>
      <ul className="drag__list">
        {rows.map((row) => (
          <li key={row.key} className="drag__row">
            <span className="drag__label">{row.label}</span>
            <DragChip
              subject={row.subject}
              format="midi"
              label={t('stems.midi')}
              title={row.label}
            />
            {row.audio && (
              <DragChip
                subject={row.subject}
                format="audio"
                label={t('stems.audio')}
                title={row.label}
              />
            )}
          </li>
        ))}
      </ul>
      {state === 'preparing' && <p className="kit-hint">{t('stems.preparing')}</p>}
      {state === 'failed' && message && (
        <p className="kit-error" role="alert">
          {message}
        </p>
      )}
    </div>
  );
}

/**
 * One thing you can pick up.
 *
 * ⚠ **`pointerdown` starts the render, `pointermove` decides it was a drag.**
 * Both are needed and neither is enough: rendering only after the threshold
 * would put a hundred milliseconds of silence between the producer's hand moving
 * and the drag beginning, and starting the drag on the press alone would turn
 * every click into one.
 */
function DragChip({
  subject,
  format,
  label,
  title,
}: {
  subject: DragSubject;
  format: DragFormat;
  /** What the chip says: the format. */
  label: string;
  /** What the drag image says: which part this is. */
  title: string;
}) {
  const begin = useDrag((s) => s.begin);
  const abandon = useDrag((s) => s.abandon);
  // ⛔ **This chip's own gesture**, handed to `begin`. Held here rather than in
  // the store because two chips must not share one: a second press while the
  // first is still preparing would otherwise clear the first one's "still held",
  // which is the exact flag that stops a released press becoming a drop.
  const gesture = useRef<Gesture | null>(null);
  const from = useRef<{ x: number; y: number } | null>(null);

  const end = () => {
    // A press that never travelled is an ordinary click, and the payload it
    // prepared has to go — otherwise the *next* drag starts from it.
    if (gesture.current) gesture.current.held = false;
    gesture.current = null;
    from.current = null;
    abandon();
  };

  return (
    <button
      type="button"
      className="drag__chip"
      // ⛔⛔ **Capture is required, and an earlier version of this file argued
      // the opposite.** The worry was that a captured pointer would stop
      // `DoDragDrop` seeing the cursor leave the plugin window. It does not:
      // capture here is the *webview's* DOM-level capture, and the plugin calls
      // Win32 `ReleaseCapture()` immediately before entering the drag loop, so
      // the OS-level capture is already gone by the time it matters.
      //
      // What dropping it actually cost: implicit capture applies only to touch
      // pointers, so with a mouse the moment the cursor leaves this ~44x22px
      // chip the button stops receiving `pointermove` **and** `pointerup`. A
      // producer dragging toward their DAW — the entire point of the feature —
      // never crossed the threshold and never released: the gesture sat
      // "Getting it ready…" for the full ten seconds and then reported a
      // timeout, with every retry in between silently swallowed.
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        from.current = { x: event.clientX, y: event.clientY };
        gesture.current = { held: true, moved: false, preview: null };
        void begin(subject, format, gesture.current);
      }}
      onPointerMove={(event) => {
        const start = from.current;
        const live = gesture.current;
        if (!start || !live || live.moved) return;
        const far =
          Math.abs(event.clientX - start.x) > THRESHOLD_PX ||
          Math.abs(event.clientY - start.y) > THRESHOLD_PX;
        if (!far) return;
        // ⚠ **Drawn here, once, and not on the press.** Every press used to draw
        // and encode ~96 KB of pixels and post them to the plugin, including the
        // presses that were ordinary clicks. The threshold is the first moment
        // the gesture is known to be a drag, and there is a whole poll interval
        // of slack before `drag_start` needs it.
        // ⛔ **The lane, not the whole pattern.** A per-lane row carries the
        // pattern whole with the lane merely named, so drawing every lane made
        // each of the eight lane chips produce a pixel-identical picture — of
        // the whole kit — while the file that dropped held one lane. That is
        // the readout-that-lies case the preview exists to prevent.
        live.preview =
          subject.kind === 'song'
            ? drawSongPreview(subject.song, title)
            : drawDragPreview(subject.patterns, title, subject.lane);
        live.moved = true;
      }}
      onPointerUp={end}
      onPointerCancel={end}
    >
      {label}
    </button>
  );
}
