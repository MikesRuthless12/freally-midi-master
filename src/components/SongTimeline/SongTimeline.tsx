/**
 * The arrangement view (TASK-063A / TASK-063B).
 *
 * A ruler carrying bar numbers and timestamps, the grid drawn under it with bar
 * lines heavier than beat lines, and the clips as objects a producer selects,
 * resizes, clones and deletes.
 *
 * ⛔ **Clips are objects, not a rendering of the structure.** Every gesture here
 * goes through `useSong`, which goes through `clips.ts` — so a resize retiles
 * the song and the ruler and the export all agree afterwards. A
 * view that drew the structure and edited a copy of it is how a timeline ends
 * up showing something the exported file does not contain.
 */

import { useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import type { Part, Section, Song } from '../../lib/ipc-types';
import { isTypingTarget } from '../../lib/keyboard';
import { useSong } from '../../state/song';
import { isSelected, partsInUse, totalBars } from './clips';
import { barLabel, barToSeconds, barToX, formatTime, gridFor } from './geometry';
import './SongTimeline.css';

/** Height of one part row, in pixels. Matches `--song-row` in the CSS. */
const ROW_HEIGHT = 44;

/**
 * ⛔ **No playhead prop, and its absence is deliberate.** TASK-063B asks for the
 * transport marker to run in this view with click-to-seek, and it is *not* built:
 * the transport reports a position in ticks of the single pattern it is playing,
 * and the plugin has no notion of playing a `Song` at all. A marker fed from the
 * pattern's clock would sit at bar 3 of a 56-bar arrangement and stay there —
 * a readout that lies, which this project has a rule about. It arrives when
 * something can actually play a song.
 */
type Props = {
  song: Song;
};

export function SongTimeline({ song }: Props) {
  const { t } = useTranslation();
  const view = useSong((s) => s.view);
  const selection = useSong((s) => s.selection);
  const select = useSong((s) => s.select);
  const selectSection = useSong((s) => s.selectSection);
  const clearSelection = useSong((s) => s.clearSelection);
  const resize = useSong((s) => s.resize);
  const clone = useSong((s) => s.clone);
  const deleteSelection = useSong((s) => s.deleteSelection);
  const copy = useSong((s) => s.copy);
  const cut = useSong((s) => s.cut);
  const paste = useSong((s) => s.paste);
  const zoomInAction = useSong((s) => s.zoomIn);
  const zoomOutAction = useSong((s) => s.zoomOut);

  const beatsPerBar = Math.max(1, song.timeSigNum);
  const bars = totalBars(song);
  const rows = partsInUse(song);
  const grid = gridFor(view, beatsPerBar);
  const width = Math.max(1, bars * view.zoom);

  // ── The ruler's labelled ticks. Only the labelled ones exist as elements;
  // the gridlines themselves are painted, not mounted (see `gridStyle`).
  const ticks = useMemo(() => {
    const out: { bar: number; label: string; time: string }[] = [];
    for (let bar = 0; bar <= bars; bar += grid.labelStep) {
      out.push({
        bar,
        label: barLabel(bar),
        time: formatTime(barToSeconds(bar, song.bpm, beatsPerBar)),
      });
    }
    return out;
  }, [bars, grid.labelStep, song.bpm, beatsPerBar]);

  // ⛔ **The grid is a repeating gradient, not one element per line.** Drawn as
  // elements it was unbounded in the song's own length: `sectionBars` accepts up
  // to `u16::MAX` and a structure may hold 64 sections, so a dataset alone could
  // ask for tens of thousands of spans mounted synchronously in one render —
  // which is the "takes the DAW down" shape this project has already been bitten
  // by. Bar and beat spacing are uniform by construction, so a gradient draws
  // the identical picture in two nodes at any length.
  const gridStyle = useMemo(() => {
    const barPx = view.zoom * grid.barStep;
    const layers = [
      `repeating-linear-gradient(to right, var(--song-bar-line) 0 1px, transparent 1px ${barPx}px)`,
    ];
    if (grid.beatStep > 0) {
      const beatPx = (view.zoom / beatsPerBar) * grid.beatStep;
      layers.push(
        `repeating-linear-gradient(to right, var(--song-beat-line) 0 1px, transparent 1px ${beatPx}px)`,
      );
    }
    // Bars last so they paint over the beat lines they coincide with.
    return { backgroundImage: layers.reverse().join(', ') };
  }, [view.zoom, grid.barStep, grid.beatStep, beatsPerBar]);

  // ── Copy / cut / paste and delete on the ordinary shortcuts (TASK-063B).
  //
  // ⛔ Bound to the timeline rather than the window, and skipped while a text
  // field has focus: the section rename is an <input>, and a window-level
  // handler would eat Ctrl+C inside it.
  const onKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      // ⛔ `isTypingTarget`, not a hand-rolled tag check. `keyboard.ts` exists
      // because this predicate had already been written three times and the
      // copies had drifted; a fourth copy here missed `<textarea>` and
      // `<select>` entirely, so the timeline would have eaten Ctrl+C from them.
      if (isTypingTarget(event.target)) return;

      const accel = event.ctrlKey || event.metaKey;
      const anchor = selection[0]?.sectionIndex ?? 0;

      if (accel && event.key === 'c') {
        copy();
        event.preventDefault();
      } else if (accel && event.key === 'x') {
        cut();
        event.preventDefault();
      } else if (accel && event.key === 'v') {
        paste(anchor);
        event.preventDefault();
      } else if (accel && event.key === 'd') {
        clone(anchor);
        event.preventDefault();
      } else if (event.key === 'Delete' || event.key === 'Backspace') {
        deleteSelection();
        event.preventDefault();
      } else if (event.key === 'Escape') {
        clearSelection();
      }
    },
    [selection, copy, cut, paste, clone, deleteSelection, clearSelection],
  );

  return (
    <div className="song" onKeyDown={onKeyDown} tabIndex={0} aria-label={t('song.timeline')}>
      <div className="song__toolbar">
        <span className="song__meta">
          {t('song.length', {
            bars,
            time: formatTime(barToSeconds(bars, song.bpm, beatsPerBar)),
          })}
        </span>
        <div className="song__zoom" role="group" aria-label={t('song.zoom')}>
          <button type="button" onClick={zoomOutAction} aria-label={t('song.zoomOut')}>
            −
          </button>
          <button type="button" onClick={zoomInAction} aria-label={t('song.zoomIn')}>
            +
          </button>
        </div>
      </div>

      <div className="song__scroller">
        <div className="song__canvas" style={{ width }}>
          {/* ── The ruler: bar numbers and timestamps over the grid. */}
          <div className="song__ruler" data-testid="song-ruler">
            {ticks.map((tick) => (
              <div
                key={tick.bar}
                className="song__tick"
                style={{ left: barToX(tick.bar, view) }}
              >
                <span className="song__bar-number">{tick.label}</span>
                <span className="song__time">{tick.time}</span>
              </div>
            ))}
          </div>

          {/* ── The section headers: name, kind and bar count (TASK-063A). */}
          <div className="song__sections">
            {song.sections.map((section, index) => (
              <SectionHeader
                key={`${section.type}-${index}`}
                section={section}
                index={index}
                left={barToX(section.startBar, view)}
                width={section.bars * view.zoom}
                selected={selection.some((c) => c.sectionIndex === index)}
                onSelect={(additive) => selectSection(index, additive)}
                onResize={(barsNext) => resize(index, barsNext)}
                onClone={() => clone(index)}
              />
            ))}
          </div>

          {/* ── The clip grid. */}
          <div
            className="song__rows"
            style={{ height: rows.length * ROW_HEIGHT }}
            onMouseDown={(event) => {
              if (event.target === event.currentTarget) clearSelection();
            }}
          >
            {/* Gridlines sit under the clips and take no pointer events, so a
                click between two clips reaches the background above. */}
            <div
              className="song__grid"
              style={gridStyle}
              data-bar-step={grid.barStep}
              data-beat-step={grid.beatStep}
              aria-hidden="true"
            />

            {rows.map((part, row) => (
              <div
                className="song__row"
                key={part}
                style={{ top: row * ROW_HEIGHT, height: ROW_HEIGHT }}
              >
                <span className="song__row-label">{t(`tabs.${part}`)}</span>
                {song.sections.map((section, index) =>
                  section.patterns[part] ? (
                    <Clip
                      key={`${part}-${index}`}
                      part={part}
                      section={section}
                      left={barToX(section.startBar, view)}
                      width={section.bars * view.zoom}
                      selected={isSelected(selection, { sectionIndex: index, part })}
                      onSelect={(additive) => select({ sectionIndex: index, part }, additive)}
                    />
                  ) : null,
                )}
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

type HeaderProps = {
  section: Section;
  index: number;
  left: number;
  width: number;
  selected: boolean;
  onSelect: (additive: boolean) => void;
  onResize: (bars: number) => void;
  onClone: () => void;
};

function SectionHeader({
  section,
  index,
  left,
  width,
  selected,
  onSelect,
  onResize,
  onClone,
}: HeaderProps) {
  const { t } = useTranslation();
  // `markers` is optional over the wire — it is skipped when empty — so an
  // older project or a hand-written song arrives without the field at all.
  const name = section.markers?.[0] ?? t(`song.kind.${section.type}`);

  return (
    <div
      className={`song__section${selected ? ' is-selected' : ''}`}
      style={{ left, width }}
      data-testid={`song-section-${index}`}
      data-kind={section.type}
      data-bars={section.bars}
    >
      <button
        type="button"
        className="song__section-name"
        onClick={(event) => onSelect(event.shiftKey || event.ctrlKey || event.metaKey)}
        onDoubleClick={onClone}
        title={t('song.sectionHint')}
      >
        <span className="song__section-title">{name}</span>
        <span className="song__section-bars">{t('song.bars', { bars: section.bars })}</span>
      </button>

      {/* Resize from *either* edge (TASK-063B). The left handle changes the
          same number as the right one — a section's length — because the
          sections tile, so there is no bar for the start edge to move to
          independently without opening a gap. */}
      <button
        type="button"
        className="song__handle song__handle--start"
        aria-label={t('song.resizeStart')}
        onClick={() => onResize(section.bars - 1)}
      />
      <button
        type="button"
        className="song__handle song__handle--end"
        aria-label={t('song.resizeEnd')}
        onClick={() => onResize(section.bars + 1)}
      />
    </div>
  );
}

type ClipProps = {
  part: Part;
  section: Section;
  left: number;
  width: number;
  selected: boolean;
  onSelect: (additive: boolean) => void;
};

function Clip({ part, section, left, width, selected, onSelect }: ClipProps) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      className={`song__clip song__clip--${part}${selected ? ' is-selected' : ''}`}
      style={{ left, width }}
      data-testid={`song-clip-${part}`}
      aria-pressed={selected}
      onClick={(event) => onSelect(event.shiftKey || event.ctrlKey || event.metaKey)}
    >
      <span className="song__clip-label">{t(`tabs.${part}`)}</span>
      {/* The transitions from TASK-066 are drawn, because they are in the
          exported file — a producer has to be able to see why the last beats of
          a section are silent. */}
      {section.dropOutBeats > 0 && (
        <span
          className="song__dropout"
          style={{ width: `${(section.dropOutBeats / (section.bars * 4)) * 100}%` }}
          title={t('song.dropOut', { beats: section.dropOutBeats })}
        />
      )}
      {section.decay && <span className="song__decay" title={t('song.decay')} />}
    </button>
  );
}
