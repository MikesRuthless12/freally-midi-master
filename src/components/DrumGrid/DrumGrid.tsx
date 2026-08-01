import { useCallback, useMemo } from 'react';
import { Volume2, VolumeX } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
import type { Pattern } from '../../lib/ipc-types';
import { toCells } from './cells';
import './DrumGrid.css';

/**
 * The generated pattern, drawn (FR-010's read-only half, US-001).
 *
 * Read-only on purpose at this stage: editing is TASK-033's piano roll and pad
 * grid, and a grid that looked editable but was not would be worse than one
 * that plainly is not.
 *
 * Laid out in 16th-note cells, which is the resolution a drum machine is
 * thought about in. Anything finer — the 32nd and triplet subdivisions inside a
 * roll — cannot have its own column without the grid becoming unreadable, so a
 * cell says how many hits landed in it and colours by the loudest. A roll then
 * looks like a roll rather than one indistinguishable tap.
 */

export function DrumGrid({ pattern, playhead }: { pattern: Pattern; playhead: number }) {
  const { t } = useTranslation();
  const seek = useSession((s) => s.seek);
  const mutedLanes = useSession((s) => s.mutedLanes);
  const setLaneMuted = useSession((s) => s.setLaneMuted);
  // ⛔ Memoised because the playhead re-renders this component on every
  // transport tick, and `toCells` walks every note to build ~1,150 fresh cell
  // objects for an 8-bar pattern. The marker is a CSS variable on a separate
  // absolutely-positioned element — none of that work affects it.
  const rows = useMemo(() => toCells(pattern), [pattern]);
  const columns = rows[0]?.cells.length ?? 0;

  // Click anywhere on the grid to play from there (TASK-041T).
  //
  // ⛔ Measured against the *track that was clicked*, not the grid. The grid
  // includes the lane-name gutter, so measuring the whole thing would put every
  // click a gutter's width late — which reads as the seek being inaccurate
  // rather than as the wrong element having been measured. `currentTarget` is
  // the track itself, so there is nothing to hold a ref to and no way for one
  // row's geometry to be used for another's.
  const seekTo = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      // ⛔ Primary button only. Bound to `onMouseDown`, every button fires it —
      // so a right-click to reach the context menu also seeked, rewinding the
      // audio thread and cutting the sampler mid-pattern while the menu the
      // user actually wanted opened over the top. Middle-click autoscroll too.
      if (event.button !== 0) return;
      const track = event.currentTarget.getBoundingClientRect();
      if (track.width === 0) return;
      void seek((event.clientX - track.left) / track.width);
    },
    [seek],
  );

  // ⛔ **Memoised for the same reason `rows` is: nothing in here reads the
  // playhead.** The marker moves 30 times a second, and every lane header and
  // all ~1,150 cell spans were being rebuilt alongside it — each header costing
  // an interpolating `t()` lookup and a fresh SVG. The marker still moves at
  // 30 Hz; the grid under it now rebuilds only when the pattern, the mutes or
  // the language actually change.
  const lanes = useMemo(
    () =>
      rows.map(({ lane, cells }) => {
        const muted = mutedLanes.includes(lane);
        const name = t(`lanes.${lane}`);
        // ⛔ **The name does not change with the state, because `aria-pressed`
        // already carries it.** WAI-ARIA's toggle-button pattern asks for one or
        // the other: swapping between "Mute…" and "Unmute…" *and* setting
        // `aria-pressed` made the announcement contradict itself — "Unmute kick
        // in the preview, toggle button, pressed" leaves a screen-reader user
        // unable to tell whether the lane is muted right now.
        const label = t('grid.muteLane', { lane: name });
        return (
          <div className="grid__row" role="row" key={lane} data-muted={muted || undefined}>
            <span className="grid__lane" role="rowheader">
              {/* ⛔ **Silences the preview, not the pattern** (FMM-S02). The
                  notes have already gone out to the host's track by the time
                  the sampler renders, so this mutes our kick without removing
                  the kick anyone routed away. The label says "preview" for
                  exactly that reason — "Mute kick" would be a lie in the one
                  place it matters. */}
              <button
                type="button"
                className="grid__mute"
                aria-pressed={muted}
                aria-label={label}
                title={label}
                onClick={() => setLaneMuted(lane, !muted)}
              >
                {muted ? (
                  <VolumeX size={12} aria-hidden="true" />
                ) : (
                  <Volume2 size={12} aria-hidden="true" />
                )}
              </button>
              <span className="grid__lanename">{name}</span>
            </span>
            <div className="grid__track" onMouseDown={seekTo}>
              {cells.map((cell, index) => (
                <span
                  key={index}
                  role="cell"
                  data-hits={cell.hits || undefined}
                  className={
                    'grid__cell' +
                    (cell.hits > 0 ? ' grid__cell--on' : '') +
                    (cell.hits > 1 ? ' grid__cell--roll' : '') +
                    (index % 4 === 0 ? ' grid__cell--beat' : '')
                  }
                  style={
                    cell.hits > 0 ? { opacity: 0.35 + (cell.velocity / 127) * 0.65 } : undefined
                  }
                />
              ))}
            </div>
          </div>
        );
      }),
    [rows, mutedLanes, setLaneMuted, seekTo, t],
  );

  return (
    <div className="grid" role="table" aria-label={t('grid.label')}>
      {/* One absolutely-positioned line rather than a class on the live cell:
          moving it is a transform, so following the playhead costs no layout
          and no React render of the grid itself. */}
      {playhead > 0 && (
        <div
          className="grid__playhead"
          style={{ '--playhead': playhead } as React.CSSProperties}
          aria-hidden="true"
        />
      )}

      {lanes}

      <p className="grid__meta">
        {t('grid.summary', {
          bars: pattern.bars,
          steps: columns,
          notes: pattern.lanes.reduce((total, track) => total + track.notes.length, 0),
        })}
      </p>
    </div>
  );
}
