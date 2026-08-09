import { useCallback, useEffect, useMemo, useState } from 'react';
import { Headphones, Lock, LockOpen, Volume2, VolumeX, Waves } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { isTypingTarget } from '../../lib/keyboard';
import { useSession } from '../../state/session';
import type { Lane, Pattern } from '../../lib/ipc-types';
import { VelocityLane } from '../PianoRoll/VelocityLane';
import { patternTicks } from '../PianoRoll/notes';
import { auditionLane } from './audition';
import {
  addFill,
  clearCell,
  cloneBar,
  reassignLane,
  toCells,
  toggleHit,
  tuplet,
  unusedLanes,
} from './cells';
import './DrumGrid.css';

/**
 * The lanes that get an "add fill" button (TASK-043H).
 *
 * ⛔ **The hats only, and that is the task rather than a shortcut.** *"The hat
 * is where trap, drill and plugg do their talking"* — the engine's `hihat.fill`
 * is authored on the hi-hat block alone, and putting the button on every lane
 * would offer a gesture with no generated counterpart, so a producer who added
 * one and then pressed Generate would watch it vanish.
 */
const FILL_LANES: readonly Lane[] = ['closedHat', 'openHat'];

/**
 * The roll palette's vocabulary: how many hits fill one 16th (TASK-043).
 *
 * ⛔ **A count, not a note-value name, because the cell is the unit.** A cell is
 * one 16th however the clip is zoomed, so "2" is a pair of 32nds, "3" is a 16th
 * triplet and "6" is a 32nd triplet — the same figures `rolls.rs` writes,
 * spelled the way the grid can actually place them. Naming them `"32"` and
 * `"16T"` here would put a second reading of the engine's roll vocabulary on the
 * page, and the two would drift the first time either changed.
 *
 * 2–8 rather than `cells.ts`'s full 2–9: past eight hits in a 16th the cell is a
 * smear at any grid width, and the keyboard chord (`Ctrl+9`) is still there for
 * anyone who wants it.
 */
const ROLL_COUNTS = [2, 3, 4, 6, 8] as const;

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
  const soloedLanes = useSession((s) => s.soloedLanes);
  const setLaneSolo = useSession((s) => s.setLaneSolo);
  const lockedLanes = useSession((s) => s.lockedLanes);
  const setLaneLocked = useSession((s) => s.setLaneLocked);
  const editPattern = useSession((s) => s.editPattern);
  /** The cell the roll palette is open over, or `null`. */
  const [palette, setPalette] = useState<{ lane: Lane; column: number } | null>(null);
  // ⛔ Memoised because the playhead re-renders this component on every
  // transport tick, and `toCells` walks every note to build ~1,150 fresh cell
  // objects for an 8-bar pattern. The marker is a CSS variable on a separate
  // absolutely-positioned element — none of that work affects it.
  const rows = useMemo(() => toCells(pattern), [pattern]);
  // ⛔ **Computed once for the whole grid, not per row.** It is a property of
  // the *pattern* — which lanes are free — and building it inside the row map
  // would walk every lane once per row for an answer that cannot differ.
  // ⛔ **The elements, not just the list.** `free` is the same in every row —
  // building ~20 `<option>`s and ~20 `t()` lookups *per row*, across up to
  // seventeen rows, is four hundred of each for one identical list.
  const freeOptions = useMemo(
    () =>
      unusedLanes(pattern).map((option) => (
        <option key={option} value={option}>
          {t(`lanes.${option}`)}
        </option>
      )),
    [pattern, t],
  );
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

  /**
   * The roll palette (TASK-043) — "choose roll type per cell", with a mouse.
   *
   * ⛔ **On right-click, over the cell, rather than as a toolbar mode.** The
   * subdivisions were already reachable by `Ctrl+3 … Ctrl+9`, and a keyboard
   * chord nobody can see is not a palette: a producer has no way to learn that
   * the grid can make triplets at all. This is the same gesture that opens the
   * transform menu in the roll, so it is one thing to learn rather than two.
   */
  const onCellMenu = useCallback((event: React.MouseEvent, lane: Lane, column: number) => {
    event.preventDefault();
    // ⚠ Not `stopPropagation`. The track's own `onMouseDown` already ignores
    // every button but the primary — see `seekTo` — so the seek this would be
    // protecting against cannot happen, and stopping it here would be a second
    // guard on a rule that is already spelled once.
    setPalette({ lane, column });
  }, []);

  // Escape and a click elsewhere are the two ways anybody dismisses a popover,
  // and a palette that survived either would sit over the grid intercepting the
  // next edit.
  useEffect(() => {
    if (palette === null) return;
    const close = () => setPalette(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    window.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', close);
    return () => {
      window.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', close);
    };
  }, [palette]);

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
        const soloed = soloedLanes.includes(lane);
        const locked = lockedLanes.includes(lane);
        // ⛔ **What the row *sounds like*, not what its own buttons say.** A
        // lane nobody muted is still silent while another lane is soloed, and a
        // row that looked live while playing nothing would be the
        // readout-that-lies failure in the one control that says what you can
        // hear. Mute wins over solo here for the same reason it does on the
        // audio thread — see `Shared::set_lane_audio`.
        const silent = muted || (soloedLanes.length > 0 && !soloed);
        const name = t(`lanes.${lane}`);
        // ⛔ **The name does not change with the state, because `aria-pressed`
        // already carries it.** WAI-ARIA's toggle-button pattern asks for one or
        // the other: swapping between "Mute…" and "Unmute…" *and* setting
        // `aria-pressed` made the announcement contradict itself — "Unmute kick
        // in the preview, toggle button, pressed" leaves a screen-reader user
        // unable to tell whether the lane is muted right now.
        const label = t('grid.muteLane', { lane: name });
        const soloLabel = t('grid.soloLane', { lane: name });
        const hearLabel = t('grid.auditionLane', { lane: name });
        const fillLabel = t('grid.addFill', { lane: name });
        const slotLabel = t('grid.reassignLane', { lane: name });
        const lockLabel = t('grid.lockLane', { lane: name });
        return (
          <div
            className="grid__row"
            role="row"
            key={lane}
            data-muted={silent || undefined}
            data-locked={locked || undefined}
            // ⛔ **`L` on the *row*, not on `window`** (TASK-044). A global
            // binding would have to guess which lane the producer meant, and
            // there is no "current lane" anywhere in this app — the grid has
            // seventeen rows and no selection model. Focus is the answer the
            // browser already has, and it bubbles from every control in the
            // header, so tabbing to a row and pressing L is unambiguous.
            onKeyDown={(event) => {
              if (event.key !== 'l' && event.key !== 'L') return;
              if (event.ctrlKey || event.metaKey || event.altKey) return;
              // ⛔ **The row holds a `<select>` since TASK-043A**, and `l` is
              // type-ahead there — so without this, picking a lane starting
              // with L toggled the lock instead. `isTypingTarget`'s selector
              // already names `select` for exactly this reason.
              if (isTypingTarget(event.target)) return;
              event.preventDefault();
              setLaneLocked(lane, !locked);
            }}
          >
            <span className="grid__lane" role="rowheader">
              {/* ⛔ **Silences the preview, not the pattern** (FMM-S02). The
                  notes have already gone out to the host's track by the time
                  the sampler renders, so this mutes our kick without removing
                  the kick anyone routed away. The label says "preview" for
                  exactly that reason — "Mute kick" would be a lie in the one
                  place it matters. */}
              <button
                type="button"
                className="grid__lock"
                aria-pressed={locked}
                aria-label={lockLabel}
                title={lockLabel}
                onClick={() => setLaneLocked(lane, !locked)}
              >
                {locked ? (
                  <Lock size={12} aria-hidden="true" />
                ) : (
                  <LockOpen size={12} aria-hidden="true" />
                )}
              </button>
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
              {/* ⛔ **Solo, beside the mute rather than folded into it**
                  (TASK-043). They answer different questions — "never play
                  this" and "only play this, for now" — and one control could
                  not express both. Like the mute it is *view and playback*
                  state: what is exported and what reaches the host's track is
                  identical either way. */}
              <button
                type="button"
                className="grid__solo"
                aria-pressed={soloed}
                aria-label={soloLabel}
                title={soloLabel}
                onClick={() => setLaneSolo(lane, !soloed)}
              >
                <Headphones size={12} aria-hidden="true" />
              </button>
              {/* ⛔ **The lane's name is the audition button** (TASK-043).
                  Mike's ask was "clicking a lane's header plays that lane's
                  sound on its own, so a producer can hear which pad they are
                  about to edit without soloing and pressing play" — so the
                  target is the *name*, the largest thing in the header, and not
                  a fourth icon nobody would find. */}
              <button
                type="button"
                className="grid__lanename"
                aria-label={hearLabel}
                title={hearLabel}
                onClick={() => void auditionLane(lane)}
              >
                {name}
              </button>
              {/* ⛔ **The slot picker (TASK-043A).** *"A slot can be
                  reassigned to any lane the kit is not already using. The
                  picker offers the unused lanes only, because two slots
                  claiming the same lane is a pattern where one of them
                  silently never sounds."*

                  ⚠ **A native `<select>`, deliberately.** It is a list of up
                  to thirty-odd names that has to be reachable by keyboard and
                  by a screen reader on three platforms; a custom popover here
                  would be a worse version of one the browser already ships.
                  It shows no visible label because the row's name is beside
                  it — the accessible name carries the whole sentence. */}
              <select
                className="grid__slot"
                aria-label={slotLabel}
                title={slotLabel}
                value={lane}
                onChange={(event) =>
                  editPattern(reassignLane(pattern, lane, event.target.value as Lane))
                }
              >
                {/* The current lane first, so the control reads as "this row is
                    a kick" rather than as an empty chooser. */}
                <option value={lane}>{name}</option>
                {freeOptions}
              </select>
              {/* The per-lane "add fill" (TASK-043H) — one press writes the
                  phrase-end figure the generator would have written, in the
                  same window `rolls::hat_fills` uses, so an added fill and a
                  generated one are the same gesture. */}
              {FILL_LANES.includes(lane) && (
                <button
                  type="button"
                  className="grid__fill"
                  aria-label={fillLabel}
                  title={fillLabel}
                  onClick={() => editPattern(addFill(pattern, lane))}
                >
                  <Waves size={12} aria-hidden="true" />
                </button>
              )}
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
                    onContextMenu={(event) => onCellMenu(event, lane, index)}
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
    [
      rows,
      freeOptions,
      mutedLanes,
      soloedLanes,
      lockedLanes,
      setLaneLocked,
      setLaneMuted,
      setLaneSolo,
      seekTo,
      onCell,
      onCellMenu,
      onCellKey,
      editPattern,
      pattern,
      t,
    ],
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

      {/* The roll palette. ⛔ **One commit per choice, so a roll is one press of
          Ctrl+Z** — `tuplet` and `clearCell` each rewrite the cell whole, which
          is the same rule `TransformMenu` keeps for the same reason. */}
      {palette !== null && (
        <div
          className="grid__palette"
          role="menu"
          aria-label={t('grid.rollLabel')}
          // The popover is inside the document-level dismiss listener's reach,
          // so its own clicks have to be kept from closing it before they land.
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            className="grid__paletteitem"
            onClick={() => {
              editPattern(clearCell(pattern, palette.lane, palette.column));
              setPalette(null);
            }}
          >
            {t('grid.rollOff')}
          </button>
          {ROLL_COUNTS.map((count) => (
            <button
              key={count}
              type="button"
              role="menuitem"
              className="grid__paletteitem"
              data-roll={count}
              onClick={() => {
                editPattern(tuplet(pattern, palette.lane, palette.column, count));
                setPalette(null);
              }}
            >
              {t('grid.rollCount', { count })}
            </button>
          ))}
        </div>
      )}

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
