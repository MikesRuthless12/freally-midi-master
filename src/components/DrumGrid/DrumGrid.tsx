import { useTranslation } from 'react-i18next';

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
  const rows = toCells(pattern);
  const columns = rows[0]?.cells.length ?? 0;

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
          <div className="grid__track">
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
