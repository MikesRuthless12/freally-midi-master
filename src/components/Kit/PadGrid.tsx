import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Play, Shuffle, X } from 'lucide-react';

import { useKit } from '../../state/kit';
import { useSession } from '../../state/session';
import { DEFAULT_PADS, useUi } from '../../state/ui';
import { ALL_LANES } from '../../state/lanes';
import { useExplorer } from '../../state/explorer';
import { auditionLane } from '../DrumGrid/audition';
import { Combo } from '../Combo/Combo';
import type { Lane } from '../../lib/ipc-types';
import './PadGrid.css';

/** Carried by a row dragged out of the sample browser. Mirrors `KitPanel`. */
const SAMPLE_TYPE = 'application/x-freally-sample';

/**
 * Module constants, and that is the whole point of them.
 *
 * ⚠ Building either of these inside the render would hand React a new array
 * every pass — see the note on the selector below, which is how this file
 * blanked the window once already.
 */
const FALLBACK_PADS: string[] = [...DEFAULT_PADS];

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
  const assign = useKit((s) => s.assign);
  const randomize = useKit((s) => s.randomize);
  const mutedLanes = useSession((s) => s.mutedLanes);
  const setLaneMuted = useSession((s) => s.setLaneMuted);
  const soloedLanes = useSession((s) => s.soloedLanes);
  const dropOn = useExplorer((s) => s.dropOn);
  const clear = useKit((s) => s.clear);
  // ⛔ **Per style** — Mike, 2026-08-09: *"the original workflows should go back
  // to exactly how they were when you left them."* Which lanes are on the pads
  // belongs to the thing being made, not to the window it is made in.
  const selectedId = useSession((s) => s.selectedId);
  // ⛔⛔ **The selector returns the stored map, and the pick happens outside it.**
  // It used to be `s.pads[selectedId] ?? []`, and the `[]` was a **new array on
  // every call** — so zustand's equality check never held, the component
  // re-rendered forever, and React gave up and painted a blank window. Mike
  // screenshotted exactly that, 2026-08-09. A selector must return something
  // stable; `s.pads` is, and the fallbacks below are module constants.
  const padsByStyle = useUi((s) => s.pads);
  const setPad = useUi((s) => s.setPad);
  // ⛔⛔ **Always eight pads, even before an artist is chosen.** This read
  // `selectedId === null ? NO_PADS : …` for one build and the whole grid
  // vanished — Mike: *"you just deleted the drum lane kit squares!!!!!"* The
  // combobox *shows* the first artist when nothing is selected, so the app
  // looked like it had one while `selectedId` was still `null`, and the pads
  // silently rendered an empty list. The kit exists whether or not a style has
  // been picked; the pads are how you reach it, so they are never absent.
  const pads = padsByStyle[selectedId ?? ''] ?? FALLBACK_PADS;
  const [over, setOver] = useState<Lane | null>(null);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!loaded) return null;

  return (
    <div className="padgrid" aria-label={t('kit.padsLabel')}>
      {pads.map((padLane, at) => {
        const lane = padLane as Lane;
        const entry = lanes.find((row) => row.lane === lane);
        // ⛔ **What the pad *sounds like*, not what its own button says.** A lane
        // nobody muted is still silent while another lane is soloed, and a pad
        // showing green while playing nothing is the readout-that-lies failure
        // in the one control that exists to say whether you can hear it. Same
        // rule the drum grid's rows already follow.
        const muted = mutedLanes.includes(lane);
        const audible = !muted && (soloedLanes.length === 0 || soloedLanes.includes(lane));
        const name = t(`lanes.${lane}`);
        const source = entry?.name ?? (entry?.shipped ? t('kit.shipped') : t('kit.silent'));

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
            // ⚠ `preventDefault` on dragOver is what makes the drop legal at
            // all; without it the browser's default is "not a drop target" and
            // the release does nothing, silently.
            onDragOver={(event) => {
              if (!event.dataTransfer.types.includes(SAMPLE_TYPE)) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = 'copy';
              if (over !== lane) setOver(lane);
            }}
            onDragLeave={() => setOver((held) => (held === lane ? null : held))}
            onDrop={(event) => {
              const path = event.dataTransfer.getData(SAMPLE_TYPE);
              setOver(null);
              if (path === '') return;
              event.preventDefault();
              // Refreshed after, because the pad's own label is the only thing
              // that says the drop landed.
              void dropOn(lane, path).then(() => refresh());
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
                options={ALL_LANES.map((id) => ({ id, name: t(`lanes.${id}`) }))}
                value={lane}
                onChange={(id) => setPad(selectedId, at, id)}
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
          </div>
        );
      })}
    </div>
  );
}
