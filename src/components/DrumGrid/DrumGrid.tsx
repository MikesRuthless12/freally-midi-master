import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Headphones, Lock, LockOpen, Volume2, VolumeX, Waves } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { isTypingTarget } from '../../lib/keyboard';
import { useSession } from '../../state/session';
import type { Lane, Pattern } from '../../lib/ipc-types';
import { Combo } from '../Combo/Combo';
import { VelocityLane } from '../PianoRoll/VelocityLane';
import { patternTicks } from '../PianoRoll/notes';
import { auditionLane } from './audition';
import {
  addFill,
  clearCell,
  clearCells,
  cloneBar,
  copyCells,
  pasteCells,
  reassignLane,
  toCells,
  toggleHit,
  tuplet,
  unusedLanes,
  cellKey,
  type CellClip,
  type CellRef,
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
 * How far a shift-drag must travel before it is a band rather than a click.
 *
 * ⚠ One number, tested in both places: while the box is being drawn and again
 * against the release point. Two thresholds would let the grid draw a band and
 * then commit a single cell.
 */
const JITTER_PX = 3;

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
  /**
   * The selected cells, keyed `lane:column` (TASK-056 #4).
   *
   * ⛔ **Cells, not note ids.** The roll's selection is a set of `NoteId`s
   * because a note there is a thing you can point at; a drum cell may hold a
   * triplet, and a producer selecting it means the figure, not one of its three
   * hits. Every operation below expands a cell to the ticks it spans, which is
   * the rule this whole module is built on.
   *
   * ⛔ **Not in the undo snapshot.** `state/editing.ts` keeps the roll's
   * selection out of `history.ts` for the same reason: a marquee across forty
   * cells is where the producer is standing, not something they made, and
   * recording it would cost forty presses of Ctrl+Z to reach the edit they
   * wanted back.
   */
  const [selection, setSelection] = useState<ReadonlyMap<string, CellRef>>(() => new Map());
  /** What Ctrl+C lifted, or `null`. Cleared when the grid unmounts, like a roll's. */
  const [clipboard, setClipboard] = useState<CellClip | null>(null);
  // ⛔ Memoised because the playhead re-renders this component on every
  // transport tick, and `toCells` walks every note to build ~1,150 fresh cell
  // objects for an 8-bar pattern. The marker is a CSS variable on a separate
  // absolutely-positioned element — none of that work affects it.
  const rows = useMemo(() => toCells(pattern), [pattern]);

  /**
   * Drop the selection when a different clip arrives.
   *
   * ⛔⛔ **A selection is drawn on a pattern, and does not survive it.** The
   * cells are keyed `lane:column`, which stays perfectly valid against a clip
   * the producer never selected anything in — so a marquee over bar 1 followed
   * by `R`, Generate or undo left the ring painted on the *new* pattern, and the
   * next Delete wiped hits nobody had picked. Keyed on the id rather than the
   * object, for the same reason the roll's framing effect is: every note edit
   * makes a new pattern object, and clearing on those would drop the selection
   * mid-gesture, which is the opposite failure.
   */
  const selectedFor = useRef(pattern.id);
  useEffect(() => {
    if (selectedFor.current === pattern.id) return;
    selectedFor.current = pattern.id;
    setSelection((current) => (current.size === 0 ? current : new Map()));
  }, [pattern.id]);
  // ⛔ **Computed once for the whole grid, not per row.** It is a property of
  // the *pattern* — which lanes are free — and building it inside the row map
  // would walk every lane once per row for an answer that cannot differ.
  // ⛔ **Computed once, not per row.** `free` is the same in every row — ~20
  // `t()` lookups *per row*, across up to seventeen rows, is four hundred of
  // them for one identical list.
  const freeOptions = useMemo(
    () => unusedLanes(pattern).map((option) => ({ id: option, name: t(`lanes.${option}`) })),
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
   * Set when a pointer gesture has already dealt with this click.
   *
   * ⛔ **The pointer decides, and the click obeys.** Reading `shiftKey` back off
   * the trailing `click` guesses at what the gesture *was*, and guesses wrong the
   * moment a hand leaves the keyboard before the mouse button.
   *
   * ⚠ **Declared above its first reader, not beside the other pointer refs.**
   * `react-hooks/immutability` reads the file in order: a ref captured by a
   * `useCallback` that appears *before* the `useRef` is just an unknown value
   * closed over by a hook argument, and writing `.current` to it is the mutation
   * that rule exists to stop.
   */
  const handled = useRef(false);

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
      // ⛔⛔ **A gesture the pointer already handled is claimed by the REF, not
      // by re-reading the modifier keys.** `click` fires after `pointerup`, and
      // a producer's hand commonly leaves the keyboard before the mouse button —
      // so a shift-click whose Shift was released a moment early arrived here
      // with `shiftKey: false`, fell through to the plain-click path, wiped the
      // selection `done` had just made and toggled a hit. A gesture documented
      // as selection-only was editing the pattern.
      if (handled.current) {
        handled.current = false;
        return;
      }

      // ⛔ **Ctrl-click selects and does not place a hit** (TASK-056 #4). Mike:
      // *"i don't care if you want me to press 'Ctrl+click' or whatever"* — so
      // the modifier is the whole gesture, and a cell that toggled a hit *and*
      // joined the selection would make every selection change the pattern.
      if (event.ctrlKey || event.metaKey) {
        setSelection((current) => {
          const next = new Map(current);
          const key = cellKey(lane, column);
          if (!next.delete(key)) next.set(key, { lane, column });
          return next;
        });
        return;
      }
      // Shift-drag drew the marquee; its closing click must not edit anything.
      if (event.shiftKey) return;
      // A plain click is a fresh start, the way it is in every editor.
      setSelection((current) => (current.size === 0 ? current : new Map()));

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
      // ⚠ The selection is only *materialised* by the branches that consume it.
      // Spreading it up here copied every selected cell on every keystroke —
      // arrows, Tab, any letter — and a full-grid marquee is over a thousand.
      const hasSelection = selection.size > 0;
      const picked = () => [...selection.values()];

      if (event.key === 'Escape' && hasSelection) {
        event.preventDefault();
        setSelection(new Map());
        return;
      }

      if (event.key === 'Backspace' || event.key === 'Delete') {
        event.preventDefault();
        // ⚠ A selection is what Delete means once there is one, and it clears in
        // one commit — see `clearCells`. Without a selection this is the single
        // cell it always was.
        if (hasSelection) {
          editPattern(clearCells(pattern, picked()));
          setSelection(new Map());
          return;
        }
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

      // Copy, paste and clone (TASK-056 #4). ⚠ Lower-cased because the browser
      // reports `C` while Shift is down, and a producer holding Shift from the
      // marquee they just drew is the common case rather than a rare one.
      const key = event.key.toLowerCase();
      if (key === 'c') {
        if (!hasSelection) return;
        event.preventDefault();
        // ⛔ **A copy that lifts nothing leaves the clipboard alone.**
        // returns null for a selection holding no notes, and writing that through
        // destroyed a block the producer had already copied — the next paste was
        // a silent no-op and the figure had to be found and copied again.
        const lifted = copyCells(pattern, picked());
        if (lifted !== null) setClipboard(lifted);
        return;
      }
      if (key === 'v') {
        if (clipboard === null) return;
        event.preventDefault();
        // ⛔ **The focused cell's column is where it lands.** There is no
        // "cursor" in this grid, and pasting back where it was copied from is a
        // no-op a producer would read as paste being broken.
        editPattern(pasteCells(pattern, clipboard, column));
        return;
      }
      if (key === 'd') {
        if (!hasSelection) return;
        event.preventDefault();
        // ⛔ **Clone lands immediately after the block, not on top of it** —
        // "clone them, copy them" means a second copy of the figure further
        // along the bar, which is how a drum machine's duplicate works.
        const lifted = copyCells(pattern, picked());
        if (lifted === null) return;
        // ⚠ The clip carries its own origin, so the block start is not derived
        // a second time here — two definitions of where a block begins is a
        // clone that lands on the wrong column the first time either changes.
        editPattern(pasteCells(pattern, lifted, lifted.fromColumn + lifted.columns));
        return;
      }

      const count = Number(event.key);
      if (!Number.isInteger(count) || count < 2 || count > 9) return;
      event.preventDefault();
      editPattern(tuplet(pattern, lane, column, count));
    },
    [editPattern, pattern, selection, clipboard],
  );

  /**
   * Right-button **drag** wipes every cell it crosses (TASK-056 #3).
   *
   * ⛔⛔ **A right-click is still the roll palette; a right-*drag* deletes.**
   * Mike asked for *"right-click and drag across the drum grid to delete —
   * multiple cells in one gesture, not one click each"*, and right-click was
   * already spoken for by TASK-043's palette. Splitting the two by whether the
   * pointer moved is the same rule the ruler uses to tell a seek from a brace,
   * so neither gesture had to be given up and there is nothing new to learn.
   *
   * ⛔ **One `editPattern` for the whole sweep**, on release. `history.ts` lists
   * `pattern` as discrete, so committing per cell would cost one Ctrl+Z per cell
   * to take back one drag — see `clearCells`.
   *
   * ⚠ **Refs, not state.** A sweep crosses cells often enough that holding it in
   * state would rebuild all ~1,150 cell spans mid-gesture, which is the cost the
   * `lanes` memo below exists to avoid.
   */
  /**
   * The gesture in flight.
   *
   * ⛔ **A union, because the two gestures share almost nothing.** A wipe
   * accumulates the cells it crosses; a marquee does not care where the pointer
   * went — the rectangle decides, and collecting cells for it was dead work that
   * also kept a second, stale idea of "what is selected" alive beside the box.
   */
  const sweep = useRef<
    | { mode: 'delete'; origin: CellRef; cells: Map<string, CellRef>; moved: boolean }
    | { mode: 'select'; start: CellRef; from: { x: number; y: number }; frame: DOMRect }
    | null
  >(null);
  /**
   * The rubber band, in coordinates relative to the grid (TASK-056 #4).
   *
   * ⛔⛔ **A marquee selects the RECTANGLE, not the path the pointer took.** The
   * first cut collected cells from `pointerenter` alone, which is a *sweep*: drag
   * from the kick's first step to the hat's last and it took the diagonal line of
   * cells the pointer happened to cross, leaving everything above and below it
   * unselected. Mike, 2026-08-11: *"like creates a box as you drag like on
   * windows and then you select multiple drum notes with drag."* So the box is
   * the selection — every cell it overlaps, whether the pointer went near it or
   * not — and drawing it is what makes that legible while the drag is happening.
   */
  const [marquee, setMarquee] = useState<{
    left: number;
    top: number;
    width: number;
    height: number;
  } | null>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const onCellDown = useCallback((event: React.PointerEvent, lane: Lane, column: number) => {
    // A fresh gesture clears the flag the last one left behind.
    handled.current = false;

    if (event.button === 2) {
      sweep.current = {
        mode: 'delete',
        origin: { lane, column },
        cells: new Map([[cellKey(lane, column), { lane, column }]]),
        moved: false,
      };
      return;
    }
    // ⚠ Shift+left is the marquee (TASK-056 #4). A plain left drag is left
    // alone: the cells tile the track, and claiming that gesture would take
    // click-to-seek off the drum grid — which is exactly what
    // `e2e/transport.spec.ts` caught the last time this file tried it.
    //
    // ⛔ **And Ctrl wins over Shift.** Ctrl+Shift+click is a producer adding one
    // more cell to a marquee they are still holding Shift for — the shortcuts
    // panel documents `Ctrl + click` as "add a cell to the selection". Arming a
    // marquee for it made the release replace the whole selection with that one
    // cell, and the trailing Ctrl-click then toggled that cell off, so the
    // sixteen-cell selection silently emptied.
    if (event.button !== 0 || !event.shiftKey || event.ctrlKey || event.metaKey) return;
    // ⚠ The grid's frame is read ONCE here rather than per pointermove. Reading
    // it while the band's inline size is being written each frame is a
    // read-after-write, and forces a synchronous layout of ~1,150 cells at
    // pointer rate. It cannot move during a drag.
    const frame = gridRef.current?.getBoundingClientRect();
    if (frame === undefined) return;
    sweep.current = {
      mode: 'select',
      start: { lane, column },
      from: { x: event.clientX, y: event.clientY },
      frame,
    };
  }, []);

  const onCellEnter = useCallback((event: React.PointerEvent, lane: Lane, column: number) => {
    const current = sweep.current;
    // ⛔ Only a wipe collects cells. A marquee's selection comes from the
    // rectangle on release, so gathering them here would be work thrown away.
    if (current === null || current.mode !== 'delete') return;
    // ⚠ `buttons` as well as the ref: a release the window listener missed —
    // the pointer left the app entirely, say — would otherwise leave a sweep
    // armed and act on cells the producer only hovered over.
    if ((event.buttons & 2) === 0) return;
    current.cells.set(cellKey(lane, column), { lane, column });
    current.moved = true;
  }, []);

  /** Every cell the rubber band overlaps, read off what is actually drawn. */
  const cellsUnder = useCallback((box: DOMRect): Map<string, CellRef> => {
    const found = new Map<string, CellRef>();
    const grid = gridRef.current;
    if (grid === null) return found;
    for (const node of grid.querySelectorAll<HTMLElement>('.grid__cell')) {
      const lane = node.dataset.lane;
      const column = Number(node.dataset.col);
      if (lane === undefined || !Number.isInteger(column)) continue;
      const at = node.getBoundingClientRect();
      // ⚠ Overlap, not containment: a band dragged across the middle of a row of
      // 14px cells would otherwise select none of them.
      const hit =
        at.left < box.right &&
        at.right > box.left &&
        at.top < box.bottom &&
        at.bottom > box.top;
      if (hit) found.set(cellKey(lane as Lane, column), { lane: lane as Lane, column });
    }
    return found;
  }, []);

  useEffect(() => {
    /**
     * The band, from its anchor to wherever the pointer is now.
     *
     * ⛔ **One derivation, used by both the drawing and the selecting.** They
     * were computed separately — and a box that is drawn from one rectangle and
     * selects from another is exactly the divergence
     * `dragging draws a box, and it selects the rectangle rather than the path`
     * exists to catch.
     */
    const bandOf = (from: { x: number; y: number }, event: PointerEvent) =>
      new DOMRect(
        Math.min(from.x, event.clientX),
        Math.min(from.y, event.clientY),
        Math.abs(event.clientX - from.x),
        Math.abs(event.clientY - from.y),
      );

    const draw = (event: PointerEvent) => {
      const current = sweep.current;
      if (current === null || current.mode !== 'select') return;
      const box = bandOf(current.from, event);
      // ⚠ Only once it is a band rather than a jitter, so a shift-click that
      // travels two pixels does not flash a box over the grid.
      if (box.width < JITTER_PX && box.height < JITTER_PX) return;
      setMarquee({
        left: box.left - current.frame.left,
        top: box.top - current.frame.top,
        width: box.width,
        height: box.height,
      });
    };

    const done = (event: PointerEvent) => {
      const current = sweep.current;
      if (current === null) return;
      sweep.current = null;
      setMarquee(null);

      if (current.mode === 'select') {
        // ⛔ The trailing click belongs to this gesture, and saying so here is what
        // stops it being re-judged from modifier keys that may already be up.
        handled.current = true;
        const box = bandOf(current.from, event);
        // ⛔ The rectangle decides, not the path. A band that never grew past a
        // jitter falls back to the single cell it started on, which is what a
        // shift-click means — and unlike the wipe, no second gesture competes
        // for it. One threshold, tested against the release point, so the
        // drawing and the committing cannot disagree about what counts as a
        // drag.
        if (box.width < JITTER_PX && box.height < JITTER_PX) {
          setSelection(
            new Map([[cellKey(current.start.lane, current.start.column), current.start]]),
          );
          return;
        }
        setSelection(cellsUnder(box));
        return;
      }

      // ⛔⛔ **The palette opens from HERE, not from the `contextmenu` event, and
      // that is a platform bug fixed rather than a preference.** This used to
      // open on `contextmenu` and let the sweep suppress it afterwards with a
      // `swallowMenu` flag — which silently assumed `contextmenu` arrives
      // *after* `pointerup`. **That is true on Windows and false on Linux and
      // macOS, where it fires on pointer DOWN.** So on those two the palette
      // opened the instant the right button went down, covered the grid, and
      // every following move landed on the menu instead of a cell: no
      // `pointerenter` fired, `moved` stayed false, and the wipe deleted
      // nothing. CI caught it on ubuntu and macOS while every local run passed,
      // because the one platform it works on is the one it was written on.
      //
      // Down arms, move collects, up decides — one ordering, every platform.
      if (!current.moved) {
        setPalette(current.origin);
        return;
      }
      editPattern(clearCells(pattern, [...current.cells.values()]));
    };

    window.addEventListener('pointermove', draw);
    window.addEventListener('pointerup', done);
    window.addEventListener('pointercancel', done);
    return () => {
      window.removeEventListener('pointermove', draw);
      window.removeEventListener('pointerup', done);
      window.removeEventListener('pointercancel', done);
    };
  }, [editPattern, pattern, cellsUnder]);

  /**
   * The roll palette (TASK-043) — "choose roll type per cell", with a mouse.
   *
   * ⛔ **On right-click, over the cell, rather than as a toolbar mode.** The
   * subdivisions were already reachable by `Ctrl+3 … Ctrl+9`, and a keyboard
   * chord nobody can see is not a palette: a producer has no way to learn that
   * the grid can make triplets at all. This is the same gesture that opens the
   * transform menu in the roll, so it is one thing to learn rather than two.
   */
  const onCellMenu = useCallback((event: React.MouseEvent) => {
    // ⛔⛔ **This handler now does one thing: stop the OS menu.** It used to be
    // where the palette opened, which made the whole gesture depend on when the
    // browser chooses to fire `contextmenu` — after `pointerup` on Windows,
    // *on pointer down* on Linux and macOS. Opening the palette at pointer-down
    // put a menu over the grid before the drag had travelled a pixel, so the
    // right-drag wipe could not collect a single cell on two of three platforms.
    // The decision moved to the `pointerup` handler above, where the gesture is
    // actually finished and its travel is known.
    //
    // ⚠ **`preventDefault` still has to happen here and nowhere else** — it is
    // the only event that can cancel the native menu, and without it the OS one
    // appears over the palette on every platform.
    event.preventDefault();
    // ⚠ Not `stopPropagation`. The track's own `onMouseDown` already ignores
    // every button but the primary — see `seekTo` — so the seek this would be
    // protecting against cannot happen, and stopping it here would be a second
    // guard on a rule that is already spelled once.
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
              // ⛔ **The row holds a slot picker since TASK-043A**, and `l` is
              // typed into it — so without this, picking a lane starting with L
              // toggled the lock instead. `isTypingTarget`'s selector names
              // `input` and `select` for exactly this reason, which is why the
              // guard still holds now that the picker is a `Combo`.
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

                  ⛔⛔ **It was a native `<select>` "deliberately", and the
                  reasoning did not survive being checked** (TASK-057). The old
                  note claimed *"a list of up to thirty-odd names … a custom
                  popover here would be a worse version of one the browser
                  already ships."* Wrong twice: the popup WebView2 ships is drawn
                  by the OS against the *window*, so the longest list in the app
                  was the control most exposed to the bug Mike screenshotted
                  rather than the one most protected from it — and the custom
                  popover was never hypothetical, because `PadGrid` has offered
                  this very list, all 37 lanes, through `Combo` for a while.

                  ▶ **What made it awkward was width, and that is now the
                  component's problem rather than this file's.** The header is
                  `--grid-label`, 5.5rem, and already carries a lock, a mute, a
                  solo, the name and a fill button, so the picker gets ~1.1rem —
                  an arrow barely wider than its own glyph. `Combo`'s menu used
                  to be measured from its field, which at that width would have
                  been a column of ellipses; it now has a floor (`MIN_MENU_PX`),
                  so a narrow trigger can open a readable list. The trigger keeps
                  the footprint the `<select>` had.
                  ⚠ It shows no visible label because the row's name is beside
                  it — the accessible name carries the whole sentence. */}
              <span className="grid__slot">
                <Combo
                  label={slotLabel}
                  // The current lane first, so the list reads as "this row is a
                  // kick" rather than as an empty chooser.
                  options={[{ id: lane, name }, ...freeOptions]}
                  value={lane}
                  onChange={(id) => editPattern(reassignLane(pattern, lane, id as Lane))}
                />
              </span>
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
              {cells.map((cell, index) => {
                // ⚠ One key and one lookup per cell rather than two of each: an
                // 8-bar pattern draws ~1,150 of these, and this file memoises
                // `lanes` precisely because per-cell cost here is real.
                const chosen = selection.has(cellKey(lane, index));
                return (
                  <span role="cell" key={index} className="grid__cellwrap">
                    <button
                      type="button"
                      aria-label={t('grid.cell', { lane: name, step: index + 1 })}
                      onClick={(event) => onCell(event, lane, index)}
                      onPointerDown={(event) => onCellDown(event, lane, index)}
                      onPointerEnter={(event) => onCellEnter(event, lane, index)}
                      onContextMenu={onCellMenu}
                      onKeyDown={(event) => onCellKey(event, lane, index)}
                      data-lane={lane}
                      data-col={index}
                      data-hits={cell.hits || undefined}
                      // ⛔ `aria-selected` as well as the class: a selection a
                      // screen reader cannot read is a selection only some of the
                      // people using this app have.
                      aria-selected={chosen || undefined}
                      data-selected={chosen || undefined}
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
                );
              })}
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
      onCellDown,
      onCellEnter,
      onCellMenu,
      onCellKey,
      selection,
      editPattern,
      pattern,
      t,
    ],
  );

  return (
    <div className="grid" role="table" aria-label={t('grid.label')} ref={gridRef}>
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

      {/* The rubber band. ⚠ `pointer-events: none` so it cannot swallow the
          pointermove that is drawing it — a box that eats its own gesture stops
          growing the moment it appears under the cursor. */}
      {marquee !== null && (
        <div
          className="grid__marquee"
          data-testid="grid-marquee"
          style={{
            left: marquee.left,
            top: marquee.top,
            width: marquee.width,
            height: marquee.height,
          }}
          aria-hidden="true"
        />
      )}

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
            // ⛔ **Collapses the roll to one hit; it does not delete it.** Mike,
            // 2026-08-09: *"when you right click on a note and you go to 'Single
            // note' it should not delete that note, it should just ensure that
            // it's a single note."* This called `clearCell`, so the one item in
            // the palette whose name promises a note was the one that removed
            // it — and a producer undoing a roll lost the hit underneath it.
            // ⚠ Deleting is still a click on the cell itself, which is where it
            // has always been.
            onClick={() => {
              editPattern(tuplet(pattern, palette.lane, palette.column, 1));
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
