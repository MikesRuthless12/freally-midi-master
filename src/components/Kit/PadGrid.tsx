import { useEffect, useState, type DragEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { CornerDownLeft, Play, Shuffle, X } from 'lucide-react';

import { PAD_TYPE, SAMPLE_TYPE, droppedSample } from '../../lib/dnd';
import { useKit } from '../../state/kit';
import { laneAudible, useSession } from '../../state/session';
import { padsOf, useUi } from '../../state/ui';
import { PAD_LANES } from '../../state/lanes';
import { useExplorer } from '../../state/explorer';
import { auditionLane } from '../DrumGrid/audition';
import { reassignLane } from '../DrumGrid/cells';
import { Combo } from '../Combo/Combo';
import type { Lane } from '../../lib/ipc-types';
import './PadGrid.css';

/**
 * The drum pads, across the top of the stage (TASK-054).
 *
 * ⛔⛔ **This replaces a list of rows nobody could find.** Mike, 2026-08-09:
 * *"the way to add drums/loops to the generators at the very top right of the
 * app should not be like that, you can barely see the 'Kick' drum lane and don't
 * know how to add samples that way."* The complaint was discoverability, not
 * styling — assignment lived in a cramped rail row, below the fold, whose only
 * instruction was a line of hint text above it.
 *
 * Each pad carries every gesture on its face: the lane's name, a dot saying
 * whether it sounds, Play to hear it, and a re-roll. Dropping a sample on it
 * assigns it.
 *
 * ⛔ **Not a dice for the re-roll**, ruled out by name. `Shuffle` says the same
 * thing without the gambling metaphor — and a dice on a *pad* would read as
 * "randomise the note", which is not what it does.
 */
export function PadGrid() {
  const { t } = useTranslation();
  const lanes = useKit((s) => s.lanes);
  const loaded = useKit((s) => s.loaded);
  const assigning = useKit((s) => s.assigning);
  const refresh = useKit((s) => s.refresh);
  const editPad = useKit((s) => s.editPad);
  const assign = useKit((s) => s.assign);
  const randomize = useKit((s) => s.randomize);
  const mutedLanes = useSession((s) => s.mutedLanes);
  const setLaneMuted = useSession((s) => s.setLaneMuted);
  const soloedLanes = useSession((s) => s.soloedLanes);
  const dropOn = useExplorer((s) => s.dropOn);
  // ⚠ **Audio only**, and asked of `selectedKind` rather than inferred. This read
  // `midiSplit === null`, which is also true for the whole window between
  // clicking a `.mid` and its split arriving — **and permanently if that split
  // fails** — so a pad offered to take a MIDI file. The same rule the drag
  // enforces with two MIME types, from the same source of truth.
  const selectedSample = useExplorer((s) => (s.selectedKind === 'audio' ? s.selected : null));
  const clear = useKit((s) => s.clear);
  // ⛔ **Per style** — Mike, 2026-08-09: *"the original workflows should go back
  // to exactly how they were when you left them."* Which lanes are on the pads
  // belongs to the thing being made, not to the window it is made in.
  const selectedId = useSession((s) => s.selectedId);
  // ⛔ **The drums clip by name, not the active tab's.** The pads sit above every
  // generator — a producer can repoint one while the Melody tab is open — and
  // `editPattern` routes on `pattern.part`, so reading "whatever is showing"
  // would drop a drum reassignment into the melody slot.
  const drums = useSession((s) => s.patterns.drums);
  const editPattern = useSession((s) => s.editPattern);
  // ⛔⛔ **The selector returns the stored map, and the pick happens outside it.**
  // It used to be `s.pads[selectedId] ?? []`, and the `[]` was a **new array on
  // every call** — so zustand's equality check never held, the component
  // re-rendered forever, and React gave up and painted a blank window. Mike
  // screenshotted exactly that, 2026-08-09. A selector must return something
  // stable; `s.pads` is, and `padsOf` answers with one frozen module constant
  // rather than a fresh copy — which is what let this file and two others stop
  // each keeping a `FALLBACK_PADS` of their own.
  const padsByStyle = useUi((s) => s.pads);
  const setPad = useUi((s) => s.setPad);
  // ⛔ Always a valid slot — `state/ui.ts::selectedPad` says why it is never null.
  const selectedPad = useUi((s) => s.selectedPad);
  const selectPad = useUi((s) => s.selectPad);
  const movePad = useUi((s) => s.movePad);
  // ⛔⛔ **Always eight pads, even before an artist is chosen.** This read
  // `selectedId === null ? NO_PADS : …` for one build and the whole grid
  // vanished — Mike: *"you just deleted the drum lane kit squares!!!!!"* The
  // combobox *shows* the first artist when nothing is selected, so the app
  // looked like it had one while `selectedId` was still `null`, and the pads
  // silently rendered an empty list. The kit exists whether or not a style has
  // been picked; the pads are how you reach it, so they are never absent.
  const pads = padsOf(padsByStyle, selectedId);
  const [over, setOver] = useState<Lane | null>(null);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!loaded) return null;

  return (
    // ⛔ **`data-picking` is what keeps the sample name clear of the buttons
    //    pinned to the pad's bottom corners** — see `.pad__source` in
    //    `PadGrid.css` for the overlap it prevents. It lives on the grid rather
    //    than on each pad because `pad__use` is drawn from a browser selection,
    //    which is one answer for all eight of them.
    <div
      className="padgrid"
      aria-label={t('kit.padsLabel')}
      data-picking={selectedSample !== null}
    >
      {pads.map((padLane, at) => {
        const lane = padLane as Lane;
        const entry = lanes.find((row) => row.lane === lane);
        // ⛔ **What the pad *sounds like*, not what its own button says.** A lane
        // nobody muted is still silent while another lane is soloed, and a pad
        // showing green while playing nothing is the readout-that-lies failure
        // in the one control that exists to say whether you can hear it. Same
        // rule the drum grid's rows already follow.
        const muted = mutedLanes.includes(lane);
        const audible = laneAudible(lane, mutedLanes, soloedLanes);
        const name = t(`lanes.${lane}`);
        const source = entry?.name ?? (entry?.shipped ? t('kit.shipped') : t('kit.silent'));

        // ⛔⛔ **Shared by the tile AND by the face button on top of it** — see
        // the `draggable` note on each. One body, because a reorder that only
        // works when you happen to grab the 3px of tile the face does not cover
        // is the bug Mike reported: *"the drum pads do not let me reorder them,
        // it just shows a selection around the drum pad."*
        const startDrag = (event: DragEvent<HTMLElement>) => {
          event.dataTransfer.setData(PAD_TYPE, String(at));
          // ⚠ `text/plain` alongside it for the reason `dnd.ts` records: some
          // WebView2 builds refuse to start a drag carrying only an
          // unrecognised type.
          event.dataTransfer.setData('text/plain', lane);
          event.dataTransfer.effectAllowed = 'move';
        };

        return (
          <div
            // ⛔ **The slot, not the lane.** Two pads may hold the same lane —
            // that is the layering case — so keying on the lane would give React
            // duplicate keys and let it reuse the wrong pad's state, which shows
            // up as a dropdown opening on the neighbour you did not click.
            key={at}
            className="pad"
            data-lane={lane}
            data-audible={audible}
            data-over={over === lane}
            data-assigned={entry?.name != null}
            data-selected={at === selectedPad}
            // ⛔⛔ **ANY press inside the pad aims the keyboard at it** — Mike,
            // 2026-08-11: *"there has to be a way to select the drum pad so you
            // know which one you are putting it into."*
            //
            // ⚠ **On the container, in the capture phase, rather than on the
            // face button** — and that is what keeps it out of the way of every
            // gesture the pad already owns. The face click still mutes, the
            // double-click still assigns, Play still auditions and the combobox
            // still opens; selecting rides along with whichever of them the
            // producer used. Hanging it on the face alone would mean the only
            // way to aim at a pad was to *mute* it.
            onPointerDownCapture={() => selectPad(at)}
            // ⛔⛔ **THE PAD IS ALSO A DRAG SOURCE, FOR REORDERING** — Mike,
            // 2026-08-11: *"you should be able to click and drag your drum pads
            // and replace the ordering, and the 'Kits' rail on the right should
            // reorder with them"* … *"and the ordering should persist from unload
            // to reload."*
            //
            // ⛔⛔ **`draggable` here AND on the face button.** Chromium will not
            // start a drag on a `<button>` and it will not walk *past* one to a
            // draggable ancestor either — a press that lands on a form control
            // is simply not a drag. `.pad__face` is `position: absolute; inset:
            // 0`, so it covers the whole tile: with `draggable` only here, every
            // grab landed on the face and nothing ever moved. That was Mike's
            // report, 2026-08-11 — *"the drum pads do not let me reorder them, it
            // just shows a selection around the drum pad"* — the ring being
            // `onPointerDownCapture` firing while the drag never began.
            //
            // ⚠ **A MIME type of its own**, so the drop handler below can tell a
            // pad from a sample coming out of the browser. Both land on the same
            // element and they mean completely different things.
            draggable
            onDragStart={startDrag}
            // ⚠ `preventDefault` on dragOver is what makes the drop legal at
            // all; without it the browser's default is "not a drop target" and
            // the release does nothing, silently.
            onDragOver={(event) => {
              const moving = event.dataTransfer.types.includes(PAD_TYPE);
              if (!moving && !event.dataTransfer.types.includes(SAMPLE_TYPE)) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = moving ? 'move' : 'copy';
              if (over !== lane) setOver(lane);
            }}
            onDragLeave={() => setOver((held) => (held === lane ? null : held))}
            onDrop={(event) => {
              setOver(null);
              // ⛔ **Reorder first**, because a pad drag also carries
              // `text/plain` and `droppedSample` would otherwise have to decide
              // it is not a path. One check, in the order the types were set.
              const from = event.dataTransfer.getData(PAD_TYPE);
              if (from !== '') {
                event.preventDefault();
                movePad(selectedId, Number(from), at);
                return;
              }
              const path = droppedSample(event.dataTransfer);
              if (path === null) return;
              event.preventDefault();
              // Refreshed after, because the pad's own label is the only thing
              // that says the drop landed.
              // ⛔ **…and its editor opens** (TASK-059), which also brings the
              // KIT panel on screen — this pad is on the stage and the editor
              // is drawn in the rail, so `editPad` shows the panel too.
              void dropOn(lane, path).then((landed) => {
                void refresh();
                // ⛔ **Only when it landed.** `dropOn` reports its own refusal through
                // `error` — a sample outside the library, say — and opening an editor
                // over a lane that still reads "Shipped" would be a panel describing a
                // drop that did not happen.
                if (landed) editPad(lane);
              });
            }}
          >
            {/* ⛔ **The pad itself mutes.** Mike: *"a way to mute/unmute by
                pressing."* Assigning moved to a double-click and to the drop,
                because muting is the gesture a producer repeats while a beat is
                playing and assigning is the one they do once. */}
            <button
              type="button"
              className="pad__face"
              aria-pressed={!audible}
              aria-label={t(audible ? 'kit.muteLane' : 'kit.unmuteLane', { lane: name })}
              title={source}
              // ⛔ **The face is the drag handle too** — it covers the tile, so
              // without this the reorder above is unreachable. See the tile's
              // `draggable` note. A click still mutes: a drag only begins once
              // the pointer moves, and `dragstart` and `click` never both fire.
              draggable
              onDragStart={startDrag}
              onClick={() => setLaneMuted(lane, !muted)}
              onDoubleClick={() => void assign(lane)}
            />

            {/* ⛔ **The name is a picker, not a label** — Mike, 2026-08-09:
                *"ensure that the names of the eight lanes are interchangeable
                with comboboxes so an end user can switch them out for other lane
                names."* Eight pads cannot cover thirty-seven lanes, so which
                eight is the producer's choice; the default is what a trap or
                drill beat is built from, and it is remembered.
                ⚠ Above the face button rather than inside it — a control inside
                a button is not a thing HTML allows, and clicks would go to
                whichever the browser felt like. */}
            <div className="pad__lane">
              <Combo
                label={t('kit.padLane', { at: at + 1 })}
                options={PAD_LANES.map((id) => ({ id, name: t(`lanes.${id}`) }))}
                value={lane}
                // ⛔⛔ **The beat's row follows the pad** — Mike, 2026-08-16:
                // *"ensure that these in the drum lanes in the pattern generator
                // are the same as the one's in the drum pad's"*, reporting that
                // pointing a pad at Mid tom *"didn't change any to mid tom"*.
                // The two controls named the same lanes from the same
                // `lanes.*` keys and wrote to different places: this one moved
                // the pad, the grid's own picker renamed the lane in the clip,
                // and neither told the other.
                //
                // ⚠ **`reassignLane` is the guard, not an afterthought.** It
                // refuses when the clip has no such lane — a pad may point at
                // one this beat never generated — and when the target lane is
                // already in the clip, which would otherwise merge two lanes'
                // hits into one and lose a part of the beat. Both cases leave
                // the pattern untouched and move only the pad, which is what
                // this control did before.
                onChange={(id) => {
                  setPad(selectedId, at, id);
                  if (drums) editPattern(reassignLane(drums, lane, id as Lane));
                }}
              />
            </div>

            {/* ⚠ **Under the picker**, and outside the mute button. It was above
                it for one build, which read as the sample naming the lane rather
                than the other way round. */}
            <span className="pad__source" title={source}>
              {source}
            </span>

            {/* ⛔ **Red or green, top right** — Mike named the position and the
                colours. `aria-hidden`, because the state is already on the
                button's `aria-pressed` and a screen reader should not hear it
                twice. */}
            <span className="pad__dot" aria-hidden="true" />

            <button
              type="button"
              className="pad__play"
              aria-label={t('kit.playLane', { lane: name })}
              title={t('kit.playLane', { lane: name })}
              onClick={() => void auditionLane(lane)}
            >
              <Play size={14} aria-hidden="true" />
            </button>

            <button
              type="button"
              className="pad__roll"
              disabled={assigning !== null}
              aria-label={t('kit.randomizeOne', { lane: name })}
              title={t('kit.randomizeOne', { lane: name })}
              onClick={() => void randomize(lane)}
            >
              <Shuffle size={12} aria-hidden="true" />
            </button>

            {/* ⛔ **Put the shipped sound back** — Mike asked whether he could
                clear a pad and return to the original, and he can: `clear` drops
                the one-shot *and* forgets the remembered path, so the lane plays
                whatever the model's own kit puts there rather than going silent.
                ⚠ Only drawn when there is something to clear, so a pad on its
                built-in sound has no control that would do nothing. */}
            {entry?.name != null && (
              <button
                type="button"
                className="pad__clear"
                disabled={assigning !== null}
                aria-label={t('kit.clearOne', { lane: name })}
                title={t('kit.clearOne', { lane: name })}
                onClick={() => void clear(lane)}
              >
                <X size={12} aria-hidden="true" />
              </button>
            )}

            {/* ⛔⛔ **The second route onto a pad, and it is not a convenience**
                (TASK-059A). A drag needs the browser and the pads on screen
                together and needs a mouse; this needs neither. It is also the
                only route that works while the rail is scrolled somewhere else.
                ⚠ **Only while a sample is selected**, so a pad carries no control
                that would have nothing to assign. `selectedSample` is null for a
                `.mid` too — a MIDI file on a drum pad is the control that can
                only fail, which `FileTree`'s two MIME types already refuse for
                the drag. */}
            {selectedSample !== null && (
              <button
                type="button"
                className="pad__use"
                disabled={assigning !== null}
                aria-label={t('kit.useSelected', { lane: name })}
                title={t('kit.useSelected', { lane: name })}
                onClick={() =>
                  void dropOn(lane, selectedSample).then((landed) => {
                    void refresh();
                    // ⛔ **Only when it landed.** `dropOn` reports its own refusal through
                    // `error` — a sample outside the library, say — and opening an editor
                    // over a lane that still reads "Shipped" would be a panel describing a
                    // drop that did not happen.
                    if (landed) editPad(lane);
                  })
                }
              >
                <CornerDownLeft size={12} aria-hidden="true" />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
