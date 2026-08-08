import { useCallback, useMemo } from 'react';
import { Volume2, VolumeX } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
import type { Lane, Pattern } from '../../lib/ipc-types';
import { VelocityLane } from '../PianoRoll/VelocityLane';
import { patternTicks } from '../PianoRoll/notes';
import { clearCell, cloneBar, toCells, toggleHit, tuplet } from './cells';
import './DrumGrid.css';

/**
 * The generated pattern, drawn and **edited** (US-001, TASK-131G).
 *
 * ⚠ **This was read-only until 2026-08-05 and its header said so** — "editing
 * is TASK-033's piano roll and pad grid, and a grid that looked editable but
 * was not would be worse than one that plainly is not". Mike asked for the
 * editing: *"we need a way to set rolls/delete rolls/set hihats/kicks/snares
 * where you want them/delete them, clone them, copy them, etc., along with
 * being able to create triplets, quintuplets"*.
 *
 * What a cell does now:
 *
 * - **Click** places a hit, or clears the cell if it has one.
 * - **Alt-click** clones the previous bar of that lane into this one.
 * - **Ctrl+3 … Ctrl+9** turn the cell into a tuplet — a triplet, a quintuplet,
 *   whatever the digit says.
 * - **Delete / Backspace** clears it.
 *
 * Laid out in 16th-note cells, which is the resolution a drum machine is
 * thought about in. Anything finer — the 32nd and triplet subdivisions inside a
 * roll — cannot have its own column without the grid becoming unreadable, so a
 * cell says how many hits landed in it and colours by the loudest.
 *
 * ⛔ **The edits work on ticks, never on cells** (`cells.ts`). A cell has
 * already thrown away where inside the 16th a hit sat, which is exactly what a
 * tuplet is made of — editing the cells and rebuilding would quantise every roll
 * in the pattern the first time anybody clicked anything.
 */

export function DrumGrid({ pattern, playhead }: { pattern: Pattern; playhead: number }) {
  const { t } = useTranslation();
  const seek = useSession((s) => s.seek);
  const mutedLanes = useSession((s) => s.mutedLanes);
  const setLaneMuted = useSession((s) => s.setLaneMuted);
  const editPattern = useSession((s) => s.editPattern);
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

  /**
   * Place or clear a hit (TASK-131G).
   *
   * ⚠ **The click still seeks, and stopping it was wrong.** The first cut put
   * `stopPropagation` on the cell so an edit would not move the playhead — which
   * silently removed click-to-seek from the drum grid entirely, because the
   * cells tile the whole track and there is no bare surface left behind them.
   * `e2e/transport.spec.ts` caught it. Both now happen: the hit lands and the
   * transport moves to it, which is where a producer is listening anyway.
   *
   * ⚠ Alt-click clones the previous bar's version of this lane into the bar the
   * cell is in — the cheapest form of "clone them, copy them" that needs no
   * selection model, and the one a drum machine actually offers.
   */
  const onCell = useCallback(
    (event: React.MouseEvent, lane: Lane, column: number) => {
      const perBar = Math.max(1, Math.round(columns / Math.max(1, pattern.bars)));
      const bar = Math.floor(column / perBar);
      // ⛔ Returns on Alt whatever the bar. `event.altKey && bar > 0` fell
      // through to `toggleHit`, so Alt-clicking a lit cell in bar 1 — where
      // there is no previous bar to clone — silently DELETED it. A gesture whose
      // whole job is to copy data must never destroy it on the edge case.
      if (event.altKey) {
        if (bar > 0) editPattern(cloneBar(pattern, lane, bar - 1, bar));
        return;
      }
      editPattern(toggleHit(pattern, lane, column));
    },
    [editPattern, pattern, columns],
  );

  /**
   * `Ctrl+3` a triplet, `Ctrl+5` a quintuplet — Mike's own example.
   *
   * ⛔ **Digits 2–9, not a fixed pair.** "triplets, quintuplets, etc." is a
   * family, and hardcoding two of them would mean coming back here for the
   * sextuplet. `Backspace` and `Delete` clear the cell, which is what a producer
   * reaches for after placing one in the wrong place.
   */
  const onCellKey = useCallback(
    (event: React.KeyboardEvent, lane: Lane, column: number) => {
      if (event.key === 'Backspace' || event.key === 'Delete') {
        event.preventDefault();
        // ⚠ `clearCell`, not a hand-rolled occupancy test. This re-derived the
        // cell span here — with `TICKS_PER_16TH` written as a bare `240` — and
        // scanned the lane twice to answer a question `cells.ts` already
        // answers, which is exactly the tick arithmetic that module exists to
        // keep in one place. It returns the pattern unchanged when the cell is
        // empty, and `editPattern` reference-compares, so the no-op is free.
        editPattern(clearCell(pattern, lane, column));
        return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      const count = Number(event.key);
      if (!Number.isInteger(count) || count < 2 || count > 9) return;
      event.preventDefault();
      editPattern(tuplet(pattern, lane, column, count));
    },
    [editPattern, pattern],
  );

  // The clip laid across whatever width the grid has — the same proportional
  // mapping the playhead uses, so the marker, the cells and the caps cannot
  // disagree about where a tick is.
  const totalTicks = patternTicks(pattern);
  const velocityX = useCallback(
    (tick: number, width: number) => (tick / totalTicks) * width,
    [totalTicks],
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
              {/* ⛔ **The cell role goes on the wrapper, the button keeps its
                  own.** These became interactive buttons and kept `role="cell"`
                  on the button itself, which OVERRIDES the implicit button role
                  — so the whole editing affordance announced as static table
                  content and a screen-reader user had no way to know Enter
                  places a hit. It was harmless on the old `<span>`, which had no
                  role to mask. Wrapping keeps the table structure the grid is
                  built on *and* exposes the control inside it. */}
              {cells.map((cell, index) => (
                <span role="cell" key={index} className="grid__cellwrap">
                  <button
                    type="button"
                    aria-label={t('grid.cell', { lane: name, step: index + 1 })}
                    onClick={(event) => onCell(event, lane, index)}
                    onKeyDown={(event) => onCellKey(event, lane, index)}
                    data-hits={cell.hits || undefined}
                    className={
                      'grid__cell' +
                      (cell.hits > 0 ? ' grid__cell--on' : '') +
                      (cell.hits > 1 ? ' grid__cell--roll' : '') +
                      (index % 4 === 0 ? ' grid__cell--beat' : '')
                    }
                    style={
                      cell.hits > 0
                        ? { opacity: 0.35 + (cell.velocity / 127) * 0.65 }
                        : undefined
                    }
                  />
                </span>
              ))}
            </div>
          </div>
        );
      }),
    [rows, mutedLanes, setLaneMuted, seekTo, onCell, onCellKey, t],
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

      {/* The velocity lane, in a row of the same shape so its caps line up with
          the cells above without either side measuring the other (TASK-041V).
          The grid itself stays read-only — a velocity is not a note, and the
          lane is what lets a producer disagree with the generator's accents
          before the pads can be edited at all. */}
      <div className="grid__velocity">
        <span aria-hidden="true" />
        <VelocityLane pattern={pattern} tracks={pattern.lanes} gutter={0} xOf={velocityX} />
      </div>

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
