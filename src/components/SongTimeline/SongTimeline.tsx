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

import { useCallback, useEffect, useMemo, useRef } from 'react';
import { Repeat, Volume2, VolumeX } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { Part, Section, Song } from '../../lib/ipc-types';
import { isTypingTarget } from '../../lib/keyboard';
import { armCurrentPattern, useSession } from '../../state/session';
import { useSong } from '../../state/song';
import { isSelected, partsInUse, totalBars } from './clips';
import { barLabel, barToSeconds, barToX, formatTime, gridFor } from './geometry';
import './SongTimeline.css';

/** Height of one part row, in pixels. Matches `--song-row` in the CSS. */
const ROW_HEIGHT = 44;

/**
 * The playhead is read from the session store rather than passed in, because
 * what it is a fraction *of* is now the whole arrangement (TASK-072).
 *
 * ⛔ This used to be deliberately absent, and the reason it could arrive is that
 * something can finally play a song: `arm_song` hands the transport the
 * flattened arrangement, so `progress` is a position through the record rather
 * than through whichever clip happened to be armed. A marker fed from one
 * pattern's clock would have sat at bar 3 of a 56-bar song and stayed there,
 * which is the readout-that-lies failure this project has a rule about.
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
  // The paste target lives in the store, not in the selection: `cut()` clears
  // the selection, so deriving it here would drop the cut clips onto section 0.
  const anchor = useSong((s) => s.anchor) ?? 0;

  const playhead = useSession((s) => s.playhead);
  const seek = useSession((s) => s.seek);
  const armSong = useSong((s) => s.armSong);
  const loopSection = useSong((s) => s.loopSection);
  const setLoopSection = useSong((s) => s.setLoopSection);
  const mutedParts = useSong((s) => s.mutedParts);
  const soloParts = useSong((s) => s.soloParts);
  const togglePartMute = useSong((s) => s.togglePartMute);
  const togglePartSolo = useSong((s) => s.togglePartSolo);

  // ⛔ **The visible tab decides what plays, and there is only one schedule.**
  // Arriving here arms the arrangement; leaving gives the transport back the
  // clip the generator tabs are showing. Without the cleanup a producer could
  // switch to Drums, press Play, and hear the whole record with the roll's
  // marker crawling across a four-bar clip — both halves looking right on their
  // own, which is the worst kind of wrong readout.
  useEffect(() => {
    armSong();
    return armCurrentPattern;
  }, [armSong, song]);

  // ⛔ **Focus has to come back here after an edit removes a clip.** A clip is a
  // <button>, so clicking one focuses it — and cut and delete then unmount the
  // very element holding focus. The browser drops focus to <body>, whose
  // keydown never reaches this handler, so the *next* shortcut silently did
  // nothing: Ctrl+X worked and the Ctrl+V after it went nowhere.
  const rootRef = useRef<HTMLDivElement>(null);

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
      // ⛔ Lower-cased, the way `App.tsx`'s undo handler already does it. With
      // caps lock on — or shift held — `event.key` is `'C'`, and every one of
      // these shortcuts silently stopped firing with nothing to explain why.
      const key = event.key.toLowerCase();

      if (accel && key === 'c') {
        copy();
        event.preventDefault();
      } else if (accel && key === 'x') {
        cut();
        rootRef.current?.focus();
        event.preventDefault();
      } else if (accel && key === 'v') {
        paste();
        event.preventDefault();
      } else if (accel && key === 'd') {
        clone(anchor);
        event.preventDefault();
      } else if (event.key === 'Delete' || event.key === 'Backspace') {
        deleteSelection();
        rootRef.current?.focus();
        event.preventDefault();
      } else if (event.key === 'Escape') {
        clearSelection();
      }
    },
    [anchor, copy, cut, paste, clone, deleteSelection, clearSelection],
  );

  return (
    <div
      className="song"
      ref={rootRef}
      onKeyDown={onKeyDown}
      tabIndex={0}
      aria-label={t('song.timeline')}
    >
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
                looping={loopSection === index}
                onSelect={(additive) => selectSection(index, additive)}
                onResize={(barsNext) => resize(index, barsNext)}
                onClone={() => clone(index)}
                onLoop={() => setLoopSection(loopSection === index ? null : index)}
              />
            ))}
          </div>

          {/* ── The transport marker (TASK-041T in this view, TASK-072).
              Drawn over the whole canvas so it crosses the ruler and every row,
              and translated rather than laid out so following it costs no
              layout — the same trick the roll's marker uses at 30 Hz. */}
          {playhead > 0 && (
            <div
              className="song__playhead"
              data-testid="song-playhead"
              style={{ transform: `translateX(${playhead * width}px)` }}
              aria-hidden="true"
            />
          )}

          {/* ── The clip grid. */}
          <div
            className="song__rows"
            style={{ height: rows.length * ROW_HEIGHT }}
            onMouseDown={(event) => {
              if (event.target !== event.currentTarget) return;
              clearSelection();
              // ⛔ Click-to-seek on the background, not on a clip: clicking a
              // clip selects it, and a gesture that both selected and moved the
              // transport would make selecting anything mid-playback jump the
              // record. Measured against the canvas the clips are laid out in,
              // so the marker lands under the pointer at any zoom.
              const track = event.currentTarget.getBoundingClientRect();
              if (track.width > 0) {
                void seek((event.clientX - track.left) / track.width);
              }
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
                {/* ⚠ The row header says *preview*: muting and soloing here
                    change what is auditioned and not the song or the exported
                    file, which is the same distinction the per-lane audio mute
                    already draws. */}
                <span className="song__row-label">
                  {t(`tabs.${part}`)}
                  <button
                    type="button"
                    className={`song__row-mute${mutedParts.includes(part) ? ' is-on' : ''}`}
                    aria-pressed={mutedParts.includes(part)}
                    aria-label={t('song.mutePart', { part: t(`tabs.${part}`) })}
                    title={t('song.previewOnly')}
                    onClick={() => togglePartMute(part)}
                  >
                    {mutedParts.includes(part) ? <VolumeX size={13} /> : <Volume2 size={13} />}
                  </button>
                  <button
                    type="button"
                    className={`song__row-solo${soloParts.includes(part) ? ' is-on' : ''}`}
                    aria-pressed={soloParts.includes(part)}
                    aria-label={t('song.soloPart', { part: t(`tabs.${part}`) })}
                    title={t('song.previewOnly')}
                    onClick={() => togglePartSolo(part)}
                  >
                    S
                  </button>
                </span>
                {song.sections.map((section, index) =>
                  section.patterns[part] ? (
                    <Clip
                      key={`${part}-${index}`}
                      part={part}
                      section={section}
                      left={barToX(section.startBar, view)}
                      width={section.bars * view.zoom}
                      selected={isSelected(selection, { sectionIndex: index, part })}
                      beatsPerBar={beatsPerBar}
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
  looping: boolean;
  onSelect: (additive: boolean) => void;
  onResize: (bars: number) => void;
  onClone: () => void;
  onLoop: () => void;
};

function SectionHeader({
  section,
  index,
  left,
  width,
  selected,
  looping,
  onSelect,
  onResize,
  onClone,
  onLoop,
}: HeaderProps) {
  const { t } = useTranslation();
  // `markers` is optional over the wire — it is skipped when empty — so an
  // older project or a hand-written song arrives without the field at all.
  const name = section.markers?.[0] ?? t(`song.kind.${section.type}`);

  return (
    <div
      className={`song__section${selected ? ' is-selected' : ''}${looping ? ' is-looping' : ''}`}
      style={{ left, width }}
      data-testid={`song-section-${index}`}
      data-kind={section.type}
      data-bars={section.bars}
      data-looping={looping ? 'true' : 'false'}
    >
      {/* Loop this section on repeat while arranging it (TASK-072). Pressing it
          again plays the record through — a toggle rather than a mode, because
          the only other way out would be a second control saying "stop". */}
      <button
        type="button"
        className={`song__loop${looping ? ' is-on' : ''}`}
        aria-pressed={looping}
        aria-label={t('song.loopSection')}
        onClick={onLoop}
      >
        <Repeat size={12} />
      </button>
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
  /** The song's own beats per bar — the drop-out is measured in them. */
  beatsPerBar: number;
  onSelect: (additive: boolean) => void;
};

function Clip({ part, section, left, width, selected, beatsPerBar, onSelect }: ClipProps) {
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
          style={{ width: `${(section.dropOutBeats / (section.bars * beatsPerBar)) * 100}%` }}
          title={t('song.dropOut', { beats: section.dropOutBeats })}
        />
      )}
      {section.decay && <span className="song__decay" title={t('song.decay')} />}
    </button>
  );
}
