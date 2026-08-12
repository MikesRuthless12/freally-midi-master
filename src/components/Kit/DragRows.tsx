import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
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
import { soundableLanes, useKit } from '../../state/kit';
import { useSong } from '../../state/song';
import { rootZoom } from '../../state/ui';
import type { Lane, Pattern } from '../../lib/ipc-types';

/**
 * Picking a generated part up and dropping it on a DAW track (TASK-063C).
 *
 * ⛔ **Not `draggable`, and not one `dragstart` handler anywhere.** An HTML5 drag
 * inside a webview is handed to the page rather than to the window server, so
 * the DAW never sees a file — the real drag is `DoDragDrop`, started by
 * `plugin/src/drag`. `state/drag.ts` carries the full note; what follows is the
 * pointer half of the same gesture.
 *
 * ⛔ **In the right rail, like everything else near the pattern.** The stage's
 * own control row is directly above the editor's bottom edge, and a row of
 * chips there costs the velocity lane — see `StemsPanel` and `layout.css` for
 * the four attempts `e2e/piano-roll.spec.ts:380` has now reverted.
 */

/**
 * One thing in the panel that can be picked up.
 *
 * Both lane lists are empty for anything that is a single file — a song, or a
 * part with one instrument — and that row renders a plain chip. A part with
 * more than one gets a menu; see [`DragMenu`].
 */
type Row = {
  key: string;
  label: string;
  subject: DragSubject;
  /**
   * The instruments this part can be taken apart into, per format.
   *
   * ⛔⛔ **The two lists differ, and Mike said why on 2026-08-06:** *"you should
   * be able to drag the midi from a lane that's been authored but has no sound,
   * but not the audio unless it has a sample associated with it."* They answer
   * different questions. MIDI asks *what was written* — notes are notes whether
   * or not this build can play them, and a producer routing them into Battery
   * wants the lane our kit happens to have no pad for. Audio asks *what can be
   * rendered*, and a lane with no sample renders silence.
   *
   * ⚠ The engine authors lanes no shipped pad plays — `snap` is one today — so
   * this is a real case rather than a defensive one. `audio/render.rs` already
   * refuses to write those files; this stops the plugin being asked.
   */
  midiLanes: Lane[];
  audioLanes: Lane[];
  /**
   * Whether this row can be dragged as audio **at all**.
   *
   * ⚠ Distinct from `audioLanes` being empty, which only means "not worth a
   * menu". A one-instrument part still drags as audio from a plain chip, and a
   * whole part nothing can play does not drag as audio from anything.
   */
  audio: boolean;
};

/**
 * Every lane this part actually wrote to, in the order the drum grid draws them.
 *
 * ⛔⛔ **THIS USED TO BE `LANE_ORDER.filter(…)`, AND THAT SILENTLY DELETED THE
 * AUDIO CHIP FROM EVERY MELODIC PART.** `LANE_ORDER` lists the *drum* lanes —
 * `cells.ts` says so in its own header — so for Melody, Chords, Countermelody
 * and Bassline the filter matched nothing and `written` came back **empty**.
 * `Row.audio` is `written.length > 0`, so those four rows rendered a MIDI chip
 * and no Audio chip at all, whatever was on the pads. Mike, 2026-08-11: *"when I
 * have samples dragged to my kit for the Melody/Chords/Counter melody, they do
 * not have an 'Audio' button able to drag audio to my DAW, but they play in the
 * generators."* The samples were never the variable; those rows could not have
 * shown an Audio chip for any kit.
 *
 * ⚠ **The ordering is what `LANE_ORDER` is still here for**, and only that: a
 * drum part reads top-to-bottom the way the grid draws it, and any lane the list
 * does not name — every melodic one — keeps its own position after them. The
 * engine's lane order is whatever it emitted, which is not the order anything
 * else in the UI uses.
 *
 * ⚠ De-duplicated, because `lanes` is a `Vec` that may carry the same `Lane`
 * twice — the layering case — and two menu entries with one name is a list where
 * neither says which is which.
 */
function writtenLanes(pattern: Pattern): Lane[] {
  const rank = new Map(LANE_ORDER.map((lane, at) => [lane, at]));
  const written = new Set(
    pattern.lanes.filter((track) => track.notes.length > 0).map((track) => track.lane),
  );
  return [...written].sort(
    (a, b) => (rank.get(a) ?? LANE_ORDER.length) - (rank.get(b) ?? LANE_ORDER.length),
  );
}

export function DragRows() {
  const { t } = useTranslation();
  const canDrag = useDrag((s) => s.canDrag);
  const patterns = useSession((s) => s.patterns);
  const song = useSong((s) => s.song);
  const state = useDrag((s) => s.state);
  const message = useDrag((s) => s.message);
  const progress = useDrag((s) => s.progress);
  const kitLanes = useKit((s) => s.lanes);
  const kitLoaded = useKit((s) => s.loaded);
  const refreshKit = useKit((s) => s.refresh);

  /**
   * ⛔⛔ **This panel loads the kit it needs, rather than hoping another one
   * already did — and that was a feature that vanished.** `useKit.refresh()` is
   * the only caller of `kit_state`, and it used to run *exclusively* from
   * `KitPanel`'s mount effect. `layout/Section.tsx` renders its children behind
   * `{open && (…)}` and the KIT section's open state is persisted in
   * localStorage, so a producer who collapsed KIT once and reloaded had
   * `lanes === []` forever: `soundable` was empty, `audio` was false on every
   * row including the song and All Parts, and **the entire Audio drag-out
   * disappeared from the Stems panel** with nothing on screen explaining why,
   * while Export still offered audio. `DragRows.test.tsx` seeds the store
   * directly, so no test could have seen it.
   */
  useEffect(() => {
    if (!kitLoaded) void refreshKit();
  }, [kitLoaded, refreshKit]);

  /**
   * Which lanes can actually make a sound.
   *
   * A shipped voice, or the producer's own one-shot over it — either is a
   * sample, and `audio/render.rs` will write a file for either.
   */
  const soundable = useMemo(() => soundableLanes(kitLanes), [kitLanes]);

  /**
   * Can this lane be heard — or do we simply not know yet?
   *
   * ⚠ **"Not asked yet" answers YES, and that is deliberate.** `loaded`
   * distinguishes an empty kit from an unread one, and treating the unread case
   * as silent is what made the chips disappear above. The cost of being wrong
   * this way is one chip offered for a fraction of a second before the reply
   * lands; the cost of being wrong the other way is a feature that is gone.
   */
  const audible = useMemo(
    () => (lane: Lane) => !kitLoaded || soundable.has(lane),
    [kitLoaded, soundable],
  );

  // ⚠ Memoised because it rebuilds a `subject` object per row, and this
  // component re-renders on every step of every drag: without it no chip's
  // props are ever stable.
  const rows = useMemo(() => {
    const built: Row[] = [];
    if (song) {
      // ⛔⛔ **An arrangement drags as audio now, and the comment here used to
      // say it could not.** It read: *"MIDI only for an arrangement, and the
      // plugin refuses the audio half rather than freezing: a song is minutes
      // long, and rendering one needs progress a producer can watch and a
      // cancel they can press."* That objection was right, and it is now
      // answered rather than avoided — `drag::Progress` reports how far the
      // render has got and stops it when the gesture is abandoned. Mike, on
      // 2026-08-06: *"we need to do progress-with-cancel so that way we can
      // drag the entire song arrangement to the DAW all at once."*
      //
      // ⚠ Still gated on the kit being able to play *something*: with no
      // soundable lane the render writes no files, and a chip that spools
      // nothing is the readout-that-lies failure this panel keeps guarding.
      built.push({
        key: 'song',
        label: t('tabs.song'),
        subject: { kind: 'song', song },
        audio: !kitLoaded || soundable.size > 0,
        midiLanes: [],
        audioLanes: [],
      });
    }

    for (const pattern of Object.values(patterns) as Pattern[]) {
      // ⚠ **A lane with no notes is not offered in either format.** Mike asked
      // for "all instruments that are used for that drum pattern" — an empty
      // lane is one the generator authored and did not write to, and a file of
      // nothing is one a producer imports and has to work out was always empty.
      const written = writtenLanes(pattern);
      // ⛔⛔ **EVERY WRITTEN LANE OFFERS BOTH FORMATS.** Mike, 2026-08-06: *"each
      // individual drum lane should be able to drag midi or audio, not just
      // midi and not just audio."*
      //
      // ⚠ **This deliberately overrides an earlier rule**, which read *"audio
      // drops the ones nothing can play"* and filtered the audio menu down to
      // `written.filter(audible)` — the lanes the loaded kit has a sample for.
      // The reasoning was sound (a lane with no pad renders a silent file) but
      // the result was a menu that offered Kick as both and Snap as MIDI only,
      // which reads as a bug rather than as a judgement. ⛔ The trade is real
      // and is Mike's call: a lane the kit cannot play now spools **silence**
      // rather than being hidden.
      const playable = written;
      // One lane is not worth a menu: the part *is* that instrument, and opening
      // a list to find a single entry repeating the row's own name is a click
      // that tells the producer nothing.
      const menu = (lanes: Lane[]) => (lanes.length > 1 ? lanes : []);
      built.push({
        key: pattern.part,
        label: t(`tabs.${pattern.part}`),
        subject: { kind: 'patterns', patterns: [pattern] },
        midiLanes: menu(written),
        audioLanes: menu(playable),
        audio: playable.length > 0,
      });
    }
    // ⛔⛔ **EVERY GENERATED PART, DRUMS INCLUDED** — Mike, 2026-08-11: *"it
    // should be all parts of the chords/melody/counter melody/basslines/drums
    // one clip after the next or ctrl should split them up into rows."*
    //
    // ⚠ **This reverses the exclusion that stood here**, which came from his own
    // 2026-08-06 sentence — *"you should also be able to drag all 4 of the other
    // generators … just the drums has to be separate because it has it's own
    // separate lanes per the generator"* — and read: *"Drums is deliberately not
    // in here. Its instruments are its own lanes, so 'all of it at once' already
    // means something different for that part."* He has now named drums in the
    // list, so it is in.
    //
    // ⛔ **This does collide with a rule he has not withdrawn**, and it is worth
    // knowing which one gave: *"there should never be one single midi file with
    // all parts of the drums draggable."* That rule is about the **Drums row's
    // own MIDI chip**, which still refuses — its menu offers one lane at a time
    // and no All Tracks. Here the drums arrive as one clip among five, which is
    // what "all parts, one after the next" has to mean if drums is one of them.
    // ▶ If it turns out he wants the drum kit left out of the MIDI half
    // specifically, this filter is the one line to change.
    //
    // ⚠ **`Cut::Parts`** on the other side: one file per part, every one of them
    // at bar 1, so the set lands as a row of clips the host can place. There is
    // no `PartsInSequence` — the end-to-end layout was reverted on 2026-08-11,
    // because offset clips arrive as a staircase of over-long ones.
    //
    // ⚠ Two or more, because with one generated part this row would be a second
    // way to drag the row directly above it.
    const everyPart = (Object.values(patterns) as Pattern[]).filter((pattern) =>
      pattern.lanes.some((track) => track.notes.length > 0),
    );
    if (everyPart.length > 1) {
      built.push({
        key: 'all-parts',
        label: t('stems.allParts'),
        subject: { kind: 'patterns', patterns: everyPart },
        midiLanes: [],
        audioLanes: [],
        // At least one of them has to be playable, or the audio chip offers a
        // drag that spools nothing.
        audio: everyPart.some((pattern) =>
          pattern.lanes.some((track) => track.notes.length > 0 && audible(track.lane)),
        ),
      });
    }

    return built;
  }, [audible, kitLoaded, patterns, song, soundable, t]);

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
            {/* ⛔ Each format asks its own question — see `Row.midiLanes`. A
                part can have eight instruments to write and only six the kit
                can play, and offering the other two as audio spools silence. */}
            <Handle row={row} format="midi" label={t('stems.midi')} lanes={row.midiLanes} />
            {row.audio && (
              <Handle
                row={row}
                format="audio"
                label={t('stems.audio')}
                lanes={row.audioLanes}
              />
            )}
          </li>
        ))}
      </ul>
      {/* ⛔⛔ **A number, once there is one to give.** A whole arrangement takes
          seconds to render and the producer is holding the mouse button down
          for all of them — "Getting it ready…" sitting there unchanged is
          indistinguishable from the plugin having hung, which is the reason the
          plugin used to refuse this request outright. A short render never gets
          here: it finishes inside one poll and `progress` stays 0, so the bar
          does not flash on and off for the common case. */}
      {state === 'preparing' && (
        <p className="kit-hint" role="status">
          {progress > 0
            ? t('stems.rendering', { percent: Math.round(progress * 100) })
            : t('stems.preparing')}
        </p>
      )}
      {state === 'failed' && message && (
        <p className="kit-error" role="alert">
          {message}
        </p>
      )}
    </div>
  );
}

/**
 * The instruments inside one part, each its own drag source.
 *
 * ⛔⛔ **Mike, 2026-08-06:** *"when you click the 'Drums' midi button, it should
 * show a menu with all instruments that are used for that drum pattern and allow
 * you to drag from just that instrument track's menu for each separate midi to
 * the DAW and the last one should say 'All Tracks'"*.
 *
 * ⚠ **This replaces the Per lane toggle as the way to reach a single lane, and
 * the reason is discoverability rather than capability.** Splitting the drums
 * into one row per lane was already built — behind a toggle in the Stems panel
 * that reads like an export preference — and Mike, looking straight at the
 * panel, reported the feature as missing. A control nobody finds is a control
 * that is not there. The toggle stays for *Export*, which is a different
 * question ("how should the files come out"), asked at a different moment.
 *
 * ⚠ **A disclosure, not an ARIA `menu`.** These entries are dragged, not
 * activated, so `role="menu"` would promise arrow-key semantics and an
 * activation this cannot honour — there is no keyboard equivalent of a drag to
 * another application. `aria-expanded` on the opener says what it does without
 * claiming what it does not.
 */
/** What both the plain chip and the menu need. Written once. */
type HandleProps = {
  row: Row;
  format: DragFormat;
  label: string;
  /** The instruments this format can be split into. Empty means a plain chip. */
  lanes: Lane[];
};

function Handle({ row, format, label, lanes }: HandleProps) {
  if (lanes.length === 0) {
    return <DragChip subject={row.subject} format={format} label={label} title={row.label} />;
  }
  return <DragMenu row={row} format={format} label={label} lanes={lanes} />;
}

function DragMenu({ row, format, label, lanes }: HandleProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLUListElement>(null);

  // ⛔ **The menu is rendered into `document.body`, and that is the fix for it
  // being cut in half.** It used to be `position: absolute` inside
  // `.rail__content`, which is `overflow-y: auto` — so the scroll container
  // cropped it and the lower lanes could not be reached at all. Mike
  // screenshotted exactly that on 2026-08-06, with Snare as the last row he
  // could see. A portal escapes the clip; nothing else about the panel changes.
  //
  // ⚠ **It still opens downward**, because he was asked about flipping it
  // upward and said *"you can do it below."* Only if it would run off the
  // bottom of the screen does it lift, and then only far enough to fit — a menu
  // that leaves the viewport is the same defect in a new place.
  // ⚠ Written straight onto the node rather than held in state: the list has to
  // be measured before it can be placed, so state would mean rendering once in
  // the wrong place and again in the right one. A layout effect runs before the
  // browser paints, so setting the style here is the only pass the user sees.
  useLayoutEffect(() => {
    const node = menu.current;
    const trigger = box.current?.getBoundingClientRect();
    if (!open || !node || !trigger) return;
    const list = node.getBoundingClientRect();

    const gap = 4;
    // Right-aligned with the chip, the way `inset-inline-end: 0` had it.
    const left = Math.max(
      gap,
      Math.min(trigger.right - list.width, window.innerWidth - list.width - gap),
    );
    const below = trigger.bottom + gap;
    const top =
      below + list.height > window.innerHeight - gap
        ? Math.max(gap, window.innerHeight - gap - list.height)
        : below;

    // ⛔ **Divided back out of the root's zoom** — `rootZoom` carries why. Every
    // measurement above is a viewport figure, and a `top`/`left` on a fixed
    // element is not. At `zoom: 1`, which is the standalone and every browser,
    // this changes nothing.
    const zoom = rootZoom();
    node.style.top = `${top / zoom}px`;
    node.style.left = `${left / zoom}px`;
    node.style.visibility = 'visible';
  }, [open, lanes.length, format]);

  // ⚠ Closes on a press anywhere else, but on `pointerdown` rather than
  // `click`: a drag that begins on another chip never produces a click, so the
  // menu would stay open under the drag that left it.
  //
  // ⛔ **Both the chip and the menu count as "inside".** The menu is portalled
  // into `document.body`, so it is no longer a DOM descendant of `box` — before
  // this second check, pressing a lane closed the menu on `pointerdown` and the
  // press never became a drag.
  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!box.current?.contains(target) && !menu.current?.contains(target)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', escape);
    };
  }, [open]);

  return (
    <div className="drag__menu" ref={box}>
      {/* ⛔⛔ **AUDIO DRAGS FROM THIS CHIP. MIDI DOES NOT, AND THAT ASYMMETRY IS
          MIKE'S RULE, NOT AN OVERSIGHT.** 2026-08-06: *"you shouldn't be able to
          drag the midi altogether for the drums, only the audio, and then the
          separate drum lanes should be draggable into the daw, there should
          never be one single midi file with all parts of the drums
          draggable."*

          A kit bounced to audio is one instrument the producer puts on one
          track; the same kit as one MIDI file is eight instruments stacked on a
          single track, which is not a thing anybody wants to receive.

          ⚠ **This chip used to be a plain `<button onClick>` in both formats**,
          so there was nothing to pick up at all — and drums is the *only* part
          that gets a menu (the only one with more than one written lane), so it
          read as "drums cannot be dragged" while every melodic part could.
          Nothing was refused and nothing was spooled; the gesture had no
          handler. `DragChip` tells a drag from a click by whether the press
          travelled, so finishing a drag never opens the menu. */}
      {format === 'audio' ? (
        <DragChip
          subject={row.subject}
          format={format}
          label={label}
          title={row.label}
          expanded={open}
          onClick={() => setOpen((was) => !was)}
        />
      ) : (
        <button
          type="button"
          className="drag__chip drag__chip--menu"
          aria-expanded={open}
          onClick={() => setOpen((was) => !was)}
        >
          {label}
        </button>
      )}
      {open &&
        createPortal(
          <ul
            className="drag__lanes"
            ref={menu}
            // ⚠ Hidden until measured — the layout effect above places it and
            // reveals it, before the browser paints. A menu that appears in the
            // wrong place and jumps is worse than one that appears once.
            style={{ visibility: 'hidden' }}
          >
            {lanes.map((lane) => (
              <li key={lane} className="drag__lane">
                <DragChip
                  subject={{ ...row.subject, lane } as DragSubject}
                  format={format}
                  label={t(`lanes.${lane}`)}
                  title={t(`lanes.${lane}`)}
                />
              </li>
            ))}
            {/* ⛔ **Last, and it is the whole kit MIXED INTO ONE FILE — AUDIO
              ONLY.** Mike, 2026-08-06: *"i want the separate audio drum lanes
              just like the midi lanes, and then an 'All Tracks' for all the
              tracks mixed for the audio lanes"* and *"i want all the clips in
              one file for 'All Tracks'"*.

              ⛔⛔ **This reverses what "All Tracks" used to do, and the reversal
              is the whole point.** It used to send `lanes: true` — one SEPARATE
              file per lane — while a second entry, "As one clip", was the mix.
              Mike read the label the other way round, pressed the mix, and
              reported the drums coming out as one track: *"not just one track
              altogether and that's it."* ▶ Measured before changing anything:
              across all 20 audio drags ever spooled to `%TEMP%\Freally MIDI
              Master\drag\`, **every single `.wav` is named for a part** and not
              one is named for a lane — so the per-lane bulk entry had never once
              been used. It was also the LAST row of a menu that was clipped from
              the bottom until `ed659dc`, so it could not be reached.

              ⚠ **The separate-file half did not go away, it moved to where he
              expects it** — the individual lane chips above, exactly as the MIDI
              menu does it. "As one clip" is gone because this entry *is* it.

              ⛔ **MIDI still gets no whole-kit entry of any kind.** Mike:
              *"there should never be one single midi file with all parts of the
              drums draggable"* and *"i just don't want an altogether midi file"*
              — one `.mid` of the kit is eight instruments stacked on one track.
              No `lanes` flag means `Cut::Parts` on the other side. */}
            {format === 'audio' && (
              <li className="drag__lane drag__lane--all">
                <DragChip
                  subject={row.subject}
                  format={format}
                  label={t('stems.allTracks')}
                  title={row.label}
                />
              </li>
            )}
          </ul>,
          document.body,
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
export function DragChip({
  subject,
  format,
  label,
  title,
  onClick,
  expanded,
}: {
  subject: DragSubject;
  format: DragFormat;
  /** What the chip says: the format. */
  label: string;
  /** What the drag image says: which part this is. */
  title: string;
  /**
   * What a press that never travelled means, if it means anything.
   *
   * ⛔ **Only fires when the gesture did not become a drag.** A chip that both
   * drags and clicks has to tell them apart, and `moved` is already the thing
   * that knows: firing on every `pointerup` would open a menu underneath the
   * producer every time they finished dragging a clip into their DAW.
   */
  onClick?: () => void;
  /** Set when this chip also opens a menu, so it can say so. */
  expanded?: boolean;
}) {
  const begin = useDrag((s) => s.begin);
  const abandon = useDrag((s) => s.abandon);
  // ⛔ **This chip's own gesture**, handed to `begin`. Held here rather than in
  // the store because two chips must not share one: a second press while the
  // first is still preparing would otherwise clear the first one's "still held",
  // which is the exact flag that stops a released press becoming a drop.
  const gesture = useRef<Gesture | null>(null);
  const from = useRef<{ x: number; y: number } | null>(null);

  // ⛔ **Set when the gesture became a drag, and read by the click that follows
  // it.** The pointer is captured, so a drag released over the DAW still
  // delivers `pointerup` — and therefore a `click` — to this button. Without
  // this, finishing a drag would open the menu under the producer every time.
  const dragged = useRef(false);

  const end = () => {
    // A press that never travelled is an ordinary click, and the payload it
    // prepared has to go — otherwise the *next* drag starts from it.
    dragged.current = gesture.current?.moved ?? false;
    if (gesture.current) gesture.current.held = false;
    gesture.current = null;
    from.current = null;
    abandon();
  };

  return (
    <button
      type="button"
      className={expanded === undefined ? 'drag__chip' : 'drag__chip drag__chip--menu'}
      aria-expanded={expanded}
      // ⚠ **The native click, minus the one that ends a drag.** Keeping this on
      // `click` rather than moving it to `pointerup` is deliberate: it is what
      // keyboard activation and `fireEvent.click` both produce, and a chip whose
      // menu only answered synthetic pointer sequences would be unreachable by
      // either.
      onClick={() => {
        if (dragged.current) {
          dragged.current = false;
          return;
        }
        onClick?.();
      }}
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
