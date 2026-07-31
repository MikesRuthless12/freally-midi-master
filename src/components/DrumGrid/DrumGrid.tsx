import { useCallback, useMemo } from 'react';
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

      {rows.map(({ lane, cells }) => (
        <div className="grid__row" role="row" key={lane}>
          <span className="grid__lane" role="rowheader">
            {t(`lanes.${lane}`)}
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
      ))}

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
