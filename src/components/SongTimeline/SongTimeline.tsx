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
import { Dices, Headphones, Lock, LockOpen, Repeat, Volume2, VolumeX } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { Part, Pattern as PatternType, Section, Song } from '../../lib/ipc-types';
import { isTypingTarget } from '../../lib/keyboard';
import { armCurrentPattern, useSession } from '../../state/session';
import { THRESHOLD_PX } from '../../state/drag';
import { lockKey, lockedRegions, useSong } from '../../state/song';
import { isSelected, partsInUse, sectionAtBar, totalBars } from './clips';
import { barLabel, barToSeconds, barToX, formatTime, gridFor, xToBar } from './geometry';
import { density, sketchGradient } from './sketch';
import { StructureChips } from './StructureChips';
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
  const move = useSong((s) => s.move);
  const deleteSelection = useSong((s) => s.deleteSelection);
  const copy = useSong((s) => s.copy);
  const cut = useSong((s) => s.cut);
  const paste = useSong((s) => s.paste);
  const zoomInAction = useSong((s) => s.zoomIn);
  const zoomOutAction = useSong((s) => s.zoomOut);
  // The paste target lives in the store, not in the selection: `cut()` clears
  // the selection, so deriving it here would drop the cut clips onto section 0.
  const anchor = useSong((s) => s.anchor) ?? 0;

  const seek = useSession((s) => s.seek);
  const armSong = useSong((s) => s.armSong);
  const loopSection = useSong((s) => s.loopSection);
  const setLoopSection = useSong((s) => s.setLoopSection);
  const selectedId = useSession((s) => s.selectedId);
  const mood = useSession((s) => s.mood);
  const reroll = useSong((s) => s.reroll);
  const audition = useSong((s) => s.audition);
  const auditionClip = useSong((s) => s.auditionClip);
  const stopAudition = useSong((s) => s.stopAudition);
  const drillInto = useSong((s) => s.drillInto);
  const exportSong = useSong((s) => s.exportSong);
  const exportStems = useSong((s) => s.exportStems);
  const exportState = useSong((s) => s.exportState);
  const exportMessage = useSong((s) => s.exportMessage);
  const structure = useSong((s) => s.structure);
  const setStructure = useSong((s) => s.setStructure);
  const locks = useSong((s) => s.locks);
  const toggleLock = useSong((s) => s.toggleLock);
  const toggleSectionLock = useSong((s) => s.toggleSectionLock);
  const toggleRowLock = useSong((s) => s.toggleRowLock);
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
  //
  // ⛔ **`song` is deliberately NOT a dependency.** It is a new object on every
  // arrangement edit, so including it made one resize fire three unordered
  // round trips — `arm_song` from `apply`, then this effect's *cleanup* arming
  // the four-bar clip, then `arm_song` again — and whichever resolved last won.
  // The store already arms on every path that changes the song; this is only
  // about arriving and leaving.
  useEffect(() => {
    armSong();
    return armCurrentPattern;
  }, [armSong]);

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

  // Which sections and rows are fully locked. Computed once per lock change
  // rather than per render: the view asks for each value three times — the
  // class, the `aria-pressed` and the icon — for every section and every row.
  const locked = useMemo(() => lockedRegions(song, locks), [song, locks]);

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
      } else if (!accel && key === 'r') {
        // ⛔ Re-roll the section the selection is in, and it is lock-respecting
        // because the store resolves the locks — the engine is handed the parts
        // to leave alone rather than the producer's clicks. `accel` is excluded
        // so Ctrl+R still reloads, which is what a producer debugging a webview
        // expects and what every browser binds it to.
        reroll(anchor, mood);
        event.preventDefault();
      } else if (event.key === 'Delete' || event.key === 'Backspace') {
        deleteSelection();
        rootRef.current?.focus();
        event.preventDefault();
      } else if (event.key === 'Escape') {
        clearSelection();
      }
    },
    [anchor, copy, cut, paste, clone, deleteSelection, clearSelection, reroll, mood],
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
        {/* Drag/export the whole arranged song (TASK-073). ⚠ The label says
            "Export", not "Drag" — an HTML5 drag inside a webview is not an OS
            file drag, and the native drag source is FMM-S03's work. A chip that
            invited a drag it cannot start would be worse than one that does not
            offer it. */}
        <button
          type="button"
          className="song__export"
          data-testid="song-export"
          disabled={exportState === 'running'}
          onClick={() => void exportSong()}
        >
          {exportState === 'running' ? t('song.exporting') : t('song.export')}
        </button>
        {/* One .mid per part, into a folder (TASK-069). ⚠ MIDI rather than
            audio: the preview kit is a drum kit, so the melodic parts have no
            voice to render through yet — the tooltip says so rather than
            letting a producer discover it by importing silence. */}
        <button
          type="button"
          className="song__export"
          data-testid="song-export-stems"
          disabled={exportState === 'running'}
          title={t('song.stemsHint')}
          onClick={() => void exportStems()}
        >
          {t('song.stems')}
        </button>
        {exportMessage !== null && (
          <span
            className={`song__export-note${exportState === 'failed' ? ' is-error' : ''}`}
            data-testid="song-export-note"
          >
            {exportState === 'done'
              ? t('song.exported', { path: exportMessage })
              : exportMessage}
          </span>
        )}

        {/* ⛔ An audition is *visible* while it lasts, and this is the way out
            of it. Without both, the transport would be looping one clip while
            the loop and solo badges on the timeline said otherwise. */}
        {audition !== null && (
          <button
            type="button"
            className="song__audition-stop"
            data-testid="song-audition-stop"
            onClick={stopAudition}
          >
            {t('song.auditionStop')}
          </button>
        )}
        <div className="song__zoom" role="group" aria-label={t('song.zoom')}>
          <button type="button" onClick={zoomOutAction} aria-label={t('song.zoomOut')}>
            −
          </button>
          <button type="button" onClick={zoomInAction} aria-label={t('song.zoomIn')}>
            +
          </button>
        </div>
      </div>

      {/* The structure the song has, and the forms the artist writes
          (TASK-070). Above the ruler because it is about the whole record. */}
      <StructureChips
        song={song}
        styleId={selectedId}
        structure={structure}
        onPick={setStructure}
      />

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
                locked={locked.sections[index] ?? false}
                onSelect={(additive) => selectSection(index, additive)}
                onResize={(barsNext) => resize(index, barsNext)}
                onClone={() => clone(index)}
                onLoop={() => setLoopSection(loopSection === index ? null : index)}
                onLock={() => toggleSectionLock(index)}
                onReroll={() => void reroll(index, mood)}
              />
            ))}
          </div>

          {/* ── The transport marker (TASK-041T in this view, TASK-072).
              Drawn over the whole canvas so it crosses the ruler and every row,
              and translated rather than laid out so following it costs no
              layout — the same trick the roll's marker uses at 30 Hz. */}
          <Playhead width={width} />

          {/* ── The clip grid. */}
          <div
            className="song__rows"
            style={{ height: rows.length * ROW_HEIGHT }}
            onMouseDown={(event) => {
              // ⛔ **`closest('.song__clip')`, not `target !== currentTarget`.**
              // That guard could never pass: every pixel of `.song__rows` is
              // covered by a `.song__row` child which is a live hit-test target
              // (only the grid, the labels, the sketches and the playhead are
              // `pointer-events: none`), so `event.target` was always a row and
              // the handler returned on its first line. Click-to-seek — the
              // thing TASK-072 advertises — and the background clear it replaced
              // were both dead, and no gate covered either.
              //
              // What the gesture actually means is "not on a clip": clicking a
              // clip selects it, and one that also moved the transport would
              // make selecting anything mid-playback jump the record.
              if ((event.target as HTMLElement).closest('.song__clip')) return;
              clearSelection();
              // Measured against the row box, which spans the same width the
              // clips and the playhead are laid out in, so the marker lands
              // under the pointer at any zoom.
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
                  {/* Lock the whole row: this part is pinned in every section
                      that plays it, so a re-roll leaves it alone (TASK-070). */}
                  <button
                    type="button"
                    className={`song__row-lock${(locked.rows[part] ?? false) ? ' is-on' : ''}`}
                    aria-pressed={locked.rows[part] ?? false}
                    aria-label={t('song.lockRow', { part: t(`tabs.${part}`) })}
                    title={t('song.lockHint')}
                    onClick={() => toggleRowLock(part)}
                  >
                    {(locked.rows[part] ?? false) ? <Lock size={12} /> : <LockOpen size={12} />}
                  </button>
                </span>
                {song.sections.map((section, index) =>
                  section.patterns[part] ? (
                    <Clip
                      key={`${part}-${index}`}
                      part={part}
                      section={section}
                      clip={song.patterns[section.patterns[part].patternId] ?? null}
                      left={barToX(section.startBar, view)}
                      width={section.bars * view.zoom}
                      selected={isSelected(selection, { sectionIndex: index, part })}
                      locked={locks.includes(lockKey({ sectionIndex: index, part }))}
                      auditioning={audition?.sectionIndex === index && audition?.part === part}
                      beatsPerBar={beatsPerBar}
                      onSelect={(additive) => select({ sectionIndex: index, part }, additive)}
                      onLock={() => toggleLock({ sectionIndex: index, part })}
                      onAudition={() => auditionClip({ sectionIndex: index, part })}
                      onDrillIn={() => drillInto({ sectionIndex: index, part })}
                      onGrab={() => {
                        // Already part of the selection: leave it alone, so
                        // dragging one of several moves all of them — which is
                        // what selecting several was for.
                        if (isSelected(selection, { sectionIndex: index, part })) return;
                        select({ sectionIndex: index, part }, false);
                      }}
                      onMoveTo={(x) => {
                        const target = sectionAtBar(song, xToBar(x, view));
                        // ⚠ Dropped past the last section, or before the first.
                        // Doing nothing is right: there is no section there to
                        // hold a clip, and inventing one would be a different
                        // gesture than the one the producer made.
                        // ⚠ `index` is where *this* clip started, and the rest
                        // of the selection moves by the same distance rather
                        // than piling onto `target` — see `moveClips`.
                        if (target !== null) move(target, index);
                      }}
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

/**
 * The transport marker, and the *only* thing here that watches the playhead.
 *
 * ⛔ **A leaf, because the subscription is what re-renders.** Read in
 * `SongTimeline` itself, the 30 Hz playhead tick reconciled the toolbar, the
 * structure row, the ruler, every section header and up to 320 clips — each
 * with its own `useTranslation`, six `t()` calls and two SVG trees — thirty
 * times a second, to move one element by a transform. `PianoRoll` and
 * `shortcuts.ts` both document reaching the same conclusion; Song Mode
 * reintroduced the pattern against a DOM tree instead of a canvas.
 */
function Playhead({ width }: { width: number }) {
  const playhead = useSession((s) => s.playhead);
  if (playhead <= 0) return null;
  return (
    <div
      className="song__playhead"
      data-testid="song-playhead"
      style={{ transform: `translateX(${playhead * width}px)` }}
      aria-hidden="true"
    />
  );
}

type HeaderProps = {
  section: Section;
  index: number;
  left: number;
  width: number;
  selected: boolean;
  looping: boolean;
  locked: boolean;
  onSelect: (additive: boolean) => void;
  onResize: (bars: number) => void;
  onClone: () => void;
  onLoop: () => void;
  onLock: () => void;
  onReroll: () => void;
};

function SectionHeader({
  section,
  index,
  left,
  width,
  selected,
  looping,
  locked,
  onSelect,
  onResize,
  onClone,
  onLoop,
  onLock,
  onReroll,
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
      {/* Lock the whole section — every clip in it — so a re-roll leaves it
          standing (TASK-070). */}
      <button
        type="button"
        className={`song__section-lock${locked ? ' is-on' : ''}`}
        aria-pressed={locked}
        aria-label={t('song.lockSection')}
        title={t('song.lockHint')}
        onClick={onLock}
      >
        {locked ? <Lock size={12} /> : <LockOpen size={12} />}
      </button>
      {/* Re-roll this section, leaving every locked clip alone (TASK-071). The
          `R` shortcut on the timeline does the same thing to the section the
          selection is in. */}
      <button
        type="button"
        className="song__section-reroll"
        aria-label={t('song.rerollSection')}
        title={t('song.rerollHint')}
        onClick={onReroll}
      >
        <Dices size={12} />
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
  /** The clip this cell plays, for its sketch. `null` for a dangling ref. */
  clip: PatternType | null;
  left: number;
  width: number;
  selected: boolean;
  locked: boolean;
  auditioning: boolean;
  /** The song's own beats per bar — the drop-out is measured in them. */
  beatsPerBar: number;
  onSelect: (additive: boolean) => void;
  onLock: () => void;
  onAudition: () => void;
  onDrillIn: () => void;
  /** The drag has started: make sure this clip is part of what will move. */
  onGrab: () => void;
  /** Dropped at `x`, in the canvas's own coordinates. */
  onMoveTo: (x: number) => void;
};

function Clip({
  part,
  section,
  clip,
  left,
  width,
  selected,
  locked,
  auditioning,
  beatsPerBar,
  onSelect,
  onLock,
  onAudition,
  onDrillIn,
  onGrab,
  onMoveTo,
}: ClipProps) {
  const { t } = useTranslation();
  /**
   * The press in progress, outside React because it changes on raw pointer
   * events and must not cost a render — the same reasoning as the stem drag's
   * `Gesture`.
   *
   * ⚠ `dragged` outlives the pointer-up on purpose: `click` fires *after* it,
   * and without this a drag would also select whatever it landed on.
   */
  const press = useRef<{ x: number; moved: boolean } | null>(null);
  const dragged = useRef(false);
  // Recomputed only when the clip itself changes: a re-roll or an edit replaces
  // the object, and a resize does not — so scrubbing a section's length does
  // not rebuild every sketch on the row.
  const sketch = useMemo(() => (clip ? sketchGradient(density(clip)) : 'none'), [clip]);

  return (
    <button
      type="button"
      className={`song__clip song__clip--${part}${selected ? ' is-selected' : ''}${
        locked ? ' is-locked' : ''
      }${auditioning ? ' is-auditioning' : ''}`}
      style={{ left, width }}
      data-testid={`song-clip-${part}`}
      data-locked={locked ? 'true' : 'false'}
      data-auditioning={auditioning ? 'true' : 'false'}
      aria-pressed={selected}
      onClick={(event) => {
        // ⛔ A drag is not a click. `click` fires after `pointerup`, so without
        // this every move would also re-select at the far end and the producer
        // would have to click away before the next gesture meant what it said.
        if (dragged.current) {
          dragged.current = false;
          return;
        }
        onSelect(event.shiftKey || event.ctrlKey || event.metaKey);
      }}
      // ⛔⛔ **Drag to rearrange (TASK-130).** Mike, 2026-08-06: *"you should be
      // able to rearrange or drag them and move them … like you would in a real
      // DAW."* Pointer events rather than HTML5 drag: this stays inside the
      // page, so there is no OS drag to start — and `state/drag.ts` explains at
      // length why an HTML5 `dragstart` in a webview is the wrong tool even
      // when the destination *is* outside.
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        press.current = { x: event.clientX, moved: false };
      }}
      onPointerMove={(event) => {
        const start = press.current;
        if (!start || start.moved) return;
        if (Math.abs(event.clientX - start.x) <= THRESHOLD_PX) return;
        start.moved = true;
        // ⚠ Captured only once the press is known to be a drag, so an ordinary
        // click never takes the pointer away from the controls on the clip.
        event.currentTarget.setPointerCapture(event.pointerId);
        // ⛔ The store moves *the selection*, so a clip dragged while something
        // else is selected has to join it first — otherwise the producer drags
        // one clip and a different one moves.
        onGrab();
      }}
      onPointerUp={(event) => {
        const start = press.current;
        press.current = null;
        if (!start?.moved) return;
        dragged.current = true;
        // ⚠ The canvas, not the row: `left` is measured from the canvas's
        // origin, which is what `xToBar` expects. A row is offset by the
        // gutter of part labels and would land every drop a few bars early.
        const canvas = event.currentTarget.closest('.song__canvas');
        if (!canvas) return;
        onMoveTo(event.clientX - canvas.getBoundingClientRect().left);
      }}
      onPointerCancel={() => {
        press.current = null;
      }}
      // Double-click drills into the part's own editor with this clip loaded,
      // and the edits write back — see the subscriber in `song.ts`.
      onDoubleClick={onDrillIn}
      title={t('song.clipHint')}
    >
      {/* The note-density sketch (TASK-070). Painted rather than mounted — see
          `sketch.ts`, and the grid above, which made the same call. */}
      <span className="song__sketch" style={{ backgroundImage: sketch }} aria-hidden="true" />
      <span className="song__clip-label">{t(`tabs.${part}`)}</span>
      {/* ⛔ A `<span role="button">` rather than a nested `<button>`: a button
          inside a button is invalid HTML and React will not render it. The clip
          itself is the button, because clicking anywhere on a clip selects it. */}
      <span
        role="button"
        tabIndex={0}
        className={`song__clip-lock${locked ? ' is-on' : ''}`}
        aria-label={t('song.lockClip', { part: t(`tabs.${part}`) })}
        aria-pressed={locked}
        title={t('song.lockHint')}
        // ⛔⛔ **The POINTER press is stopped here too, not only the click.**
        // The clip's drag-to-move handlers are `onPointerDown`/`Move`/`Up` on
        // the button this sits inside, and stopping `click` does nothing about
        // those: a press on the lock with a few pixels of hand travel — six is
        // the threshold — crossed it, captured the pointer, called `onGrab`
        // (which replaces the whole selection with this one clip) and then
        // moved the clip if those pixels crossed a section boundary. One press
        // on a padlock did all three, from a gesture the comment below says
        // must do only the first.
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          // Selecting is what a click on the clip does; locking must not also
          // do it, or every lock would move the selection under the producer.
          event.stopPropagation();
          onLock();
        }}
        onKeyDown={(event) => {
          if (event.key !== 'Enter' && event.key !== ' ') return;
          event.stopPropagation();
          event.preventDefault();
          onLock();
        }}
      >
        {locked ? <Lock size={11} /> : <LockOpen size={11} />}
      </span>
      {/* Solo-audition this one cell (TASK-071). Its own control rather than the
          bare click, because a click already selects — and arming one looping
          clip invisibly would leave the transport playing something the
          timeline's loop and solo badges say it is not. */}
      <span
        role="button"
        tabIndex={0}
        className={`song__clip-audition${auditioning ? ' is-on' : ''}`}
        aria-label={t('song.auditionClip', { part: t(`tabs.${part}`) })}
        aria-pressed={auditioning}
        // The same reason as the lock above: a press here must not become a drag.
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          onAudition();
        }}
        onKeyDown={(event) => {
          if (event.key !== 'Enter' && event.key !== ' ') return;
          event.stopPropagation();
          event.preventDefault();
          onAudition();
        }}
      >
        <Headphones size={11} />
      </span>
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
