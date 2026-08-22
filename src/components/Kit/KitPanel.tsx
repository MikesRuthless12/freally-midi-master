import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Dices, FolderPlus, Play, SlidersHorizontal, X } from 'lucide-react';

import { canSound, useKit } from '../../state/kit';
import { SAMPLE_TYPE, droppedSample } from '../../lib/dnd';
import { auditionLane } from '../DrumGrid/audition';
import { PadEditor } from './PadEditor';
import { SavedKits } from './SavedKits';
import { useSession } from '../../state/session';
import { padsOf, useUi } from '../../state/ui';
import type { Lane } from '../../lib/ipc-types';

/**
 * The type a browser row carries while it is being dragged.
 *
 * ⚠ **Private, and checked rather than assumed.** `text/plain` is also set —
 * some WebView2 builds refuse to start a drag that carries only an unrecognised
 * MIME type — but accepting *any* plain text here would make every stray drag
 * from outside the page look like a sample and fail at the loader instead of at
 * the drop.
 */

/**
 * The KIT panel: what each lane plays, and how to put your own sample on it.
 *
 * ⛔ **Every word here comes from `kit_state`.** What this replaces rendered
 * eight hardcoded `disabled` buttons and a static "No kit yet" while a
 * twelve-pad kit was loaded and audibly playing (TASK-136) — Mike found it in
 * Ableton inside a minute. The lesson worth keeping is not "write a better
 * string": it is that a panel with no data behind it will always eventually say
 * something untrue, and the fix is the wiring.
 *
 * ⛔ **A row per lane, not a grid of squares.** The old square pads had room for
 * an index and nothing else. What a producer needs to see is which lane it is,
 * what is playing it, and whether that is theirs — three facts that do not fit
 * in a 44px square, and the panel was numbering pads `1..8` precisely because it
 * had nothing else to put there.
 */
export function KitPanel() {
  const { t } = useTranslation();
  const allLanes = useKit((s) => s.lanes);
  const loaded = useKit((s) => s.loaded);
  const assigning = useKit((s) => s.assigning);
  // ⚠ **Anything holding the plugin dialog slot**, not just a per-lane
  // assignment: a batch import holds it too, and it belongs to no lane.
  const busy = useKit((s) => s.assigning !== null || s.importing);
  const error = useKit((s) => s.error);
  const refresh = useKit((s) => s.refresh);
  const assign = useKit((s) => s.assign);
  const clear = useKit((s) => s.clear);
  const randomize = useKit((s) => s.randomize);
  const addMany = useKit((s) => s.addMany);
  const imported = useKit((s) => s.imported);
  const dismissImport = useKit((s) => s.dismissImport);
  const dropSample = useKit((s) => s.drop);
  // Which row the pointer is currently over, so the target is visible before
  // the producer lets go. Local: it is a property of this gesture, not of the
  // kit, and nothing outside this panel can act on it.
  const [over, setOver] = useState<Lane | null>(null);
  // ⛔ **In the store, not local** (TASK-059). Three gestures open this editor
  // and only one of them is in this file — see `useKit.editingPad`.
  const editing = useKit((s) => s.editingPad);
  const setEditing = useKit((s) => s.editPad);

  /**
   * The eight lanes on the pads first, in pad order, then everything else.
   *
   * ⛔⛔ **Mike, 2026-08-11:** *"on the right side for the 'Kit' the top 8 names
   * should always be the one's that are visible at the time, and when you swap
   * them for the others, then those names should be at the top 8 in the same
   * order as the drum pads are."*
   *
   * ▶ The list arrives in the plugin's own order — thirty-odd lanes, of which
   * eight are on screen as pads and the rest are not. So finding the row for the
   * pad you are looking at meant scrolling past twenty you are not using, and
   * changing a pad's lane moved its row somewhere unrelated. Sorting by the pad
   * layout makes the panel and the grid read the same way round.
   *
   * ⚠ **A stable sort on the pad index**, so the remaining lanes keep the
   * plugin's ordering rather than being shuffled into whatever the comparison
   * happens to do with two equal keys.
   *
   * ⚠ A lane on two pads — the layering case `PAD_LIMIT` allows — sorts by its
   * *first* pad, because `indexOf` answers the first and one row cannot be in two
   * places.
   */
  // ⛔⛔ **The selector returns the stored MAP; the pick happens outside it.**
  // A selector that built the default layout itself would hand back a **new
  // array on every call**, zustand's equality check would never hold, the
  // component would re-render forever and React would give up and paint a blank
  // panel — Mike screenshotted exactly that on 2026-08-09, and it was
  // reintroduced here the first time this sort was written. `s.pads` is stable
  // and `padsOf` answers with one frozen constant.
  const selectedId = useSession((s) => s.selectedId);
  const padsByStyle = useUi((s) => s.pads);
  const pads = padsOf(padsByStyle, selectedId);
  const lanes = useMemo(() => {
    // ⚠ A lookup table rather than `pads.indexOf` inside the comparator: a sort
    // of ~30 lanes asks the comparator ~150 times and each call scanned the pad
    // array twice.
    //
    // ⛔⛔ **THE FIRST SLOT WINS, WHICH IS WHAT `indexOf` ANSWERED.** Two pads may
    // hold the same lane — `PAD_LIMIT` allows it so a snare can be layered — and
    // a plain `new Map(pads.map(...))` keeps the *last* of a duplicate key. Point
    // pads 1 and 7 at the kick and the KIT list sorted Kick seventh while its pad
    // was first, which is the ordering Mike asked for failing on the one case
    // this comment claims to cover: *"the top 8 names should … be at the top 8 in
    // the same order as the drum pads are."*
    const rank = new Map<string, number>();
    pads.forEach((lane, at) => {
      if (!rank.has(lane)) rank.set(lane, at);
    });
    const at = (lane: Lane) => rank.get(lane) ?? pads.length;
    return [...allLanes].sort((a, b) => at(a.lane) - at(b.lane));
  }, [allLanes, pads]);

  // Read once when the panel mounts. `Section` unmounts a collapsed panel's
  // content, so reopening it re-reads — which is what keeps it in step with an
  // assignment made from somewhere else.
  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!loaded) {
    return <div className="kit-drop">{t('kit.loading')}</div>;
  }

  if (lanes.length === 0) {
    // ⚠ The honest empty state, and the only one. It means the plugin could not
    // decode its own kit — which `audio::preview_kit` logs and which leaves the
    // plugin silent — rather than "no kit has been chosen", which was never a
    // thing this product had.
    return <div className="kit-drop">{t('kit.noneLoaded')}</div>;
  }

  return (
    <>
      {/* ⚠ `kit-hint`, not the shared `rail__hint`. The roster's "nobody works
          in this genre" hint uses that class and `e2e/cross-filter.spec.ts`
          locates it by class alone — a second `.rail__hint` in the same rail
          made that locator ambiguous and failed two specs that have nothing to
          do with the kit. */}
      <p className="kit-hint">{t('kit.assignHint')}</p>
      {/* ⛔ **One dice for the whole kit** (TASK-050A). Re-rolls every
          unlocked pad from the folder the browser is showing, in one gesture and
          one handoff to the audio thread — a dozen separate assignments would
          be a dozen audible cuts. */}
      <button
        type="button"
        className="btn-ghost kit-dice"
        disabled={busy}
        aria-label={t('kit.randomize')}
        title={t('kit.randomize')}
        onClick={() => void randomize(null)}
      >
        <Dices size={14} aria-hidden="true" /> {t('kit.randomize')}
      </button>
      {/* ⛔ **A whole pack in one dialog** (TASK-049). Each file lands on the
          pad its own name names — `roles::guess`, which shipped in 2026-08-09
          with nothing calling it. Beside the dice because both fill several
          pads at once; the difference is that this one is the producer's own
          selection rather than a draw from the folder being browsed. */}
      <button
        type="button"
        className="btn-ghost kit-dice"
        disabled={busy}
        aria-label={t('kit.addMany')}
        title={t('kit.addMany')}
        onClick={() => void addMany()}
      >
        <FolderPlus size={14} aria-hidden="true" /> {t('kit.addMany')}
      </button>

      <ul className="kit-lanes" aria-label={t('kit.lanesLabel')}>
        {lanes.map((entry) => {
          const busy = assigning === entry.lane;
          const source = entry.name
            ? entry.name
            : entry.shipped
              ? t('kit.shipped')
              : t('kit.silent');
          return (
            <li
              key={entry.lane}
              className="kit-lane"
              data-lane={entry.lane}
              data-assigned={entry.name !== null}
              // ⚠ **The shared predicate, not a second spelling of it.** This
              // asked `!shipped && name === null` while `DragRows` asked
              // `shipped || path !== null` — the same question keyed on
              // different fields, with nothing making them move together. A
              // lane could read as playable here and be hidden from the drag
              // menu, and no test could catch the disagreement because neither
              // file knew about the other.
              data-silent={!canSound(entry)}
              data-over={over === entry.lane}
              // ⛔ **The third door into one assignment model** (TASK-132). The
              // combobox and the file dialog already led here; Mike, 2026-08-06:
              // *"when we do the 'File Explorer' then we will be able to drop
              // samples on the generators and drum lanes."* All three end at
              // `OneShots::restore`, which is the tested no-dialog load a
              // reopened project already uses — a second loader for the drop
              // would be a second set of the same rules to keep in agreement.
              //
              // ⚠ **`preventDefault` on dragOver is what makes a drop legal at
              // all.** Without it the browser's default is "not a drop target"
              // and the release does nothing, silently.
              onDragOver={(event) => {
                if (!event.dataTransfer.types.includes(SAMPLE_TYPE)) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = 'copy';
                if (over !== entry.lane) setOver(entry.lane);
              }}
              onDragLeave={() => setOver((lane) => (lane === entry.lane ? null : lane))}
              onDrop={(event) => {
                const path = droppedSample(event.dataTransfer);
                setOver(null);
                if (path === null) return;
                event.preventDefault();
                // ⚠ Refreshed after, because the row's own label is what says
                // whether the drop landed — the panel is the only feedback the
                // producer gets, and it would otherwise still read "Shipped".
                // ⛔ **…and open its editor** (TASK-059): the gesture is
                // "imports, assigns, and opens the per-one-shot editor". A
                // sample that lands with no way to shape it is where this
                // stopped before the editor existed.
                void dropSample(entry.lane, path).then((landed) => {
                  // ⛔ **Only when it landed.** `dropOn` reports its own refusal through
                  // `error` — a sample outside the library, say — and opening an editor
                  // over a lane that still reads "Shipped" would be a panel describing a
                  // drop that did not happen.
                  if (landed) setEditing(entry.lane);
                });
              }}
            >
              <button
                type="button"
                className="kit-lane__pad"
                // ⚠ Disabled only while *this* panel has a dialog open, and it
                // has to be: two native dialogs from one plugin is a window a
                // producer cannot explain, and the plugin refuses the second
                // one anyway.
                disabled={busy}
                onClick={() => void assign(entry.lane)}
              >
                <span className="kit-lane__name">{t(`lanes.${entry.lane}`)}</span>
                <span className="kit-lane__source">{busy ? t('kit.choosing') : source}</span>
              </button>

              {/* ⛔⛔ **HEAR THIS LANE'S SAMPLE ON ITS OWN** — Mike, 2026-08-11:
                  *"the melody/chords/basslines/counter melody should be able to
                  play back their samples with a play button as well, not just
                  with the generated playback pattern"* and *"you should be able
                  to hear just the sample or one shot you are using."*

                  ▶ **The plugin could already do this; there was no button.**
                  `Audition::Lane` resolves through `Kit::pad_for` and triggers
                  the pad **as sampled** — zero transposition, no generated part,
                  which is exactly "just the sample". The drum lanes have reached
                  it from `PadGrid` and the drum grid's row headers since
                  TASK-043; the melodic lanes live only in this list, and this
                  list had no way in.

                  ⚠ **Only where something can sound.** `canSound` is the shared
                  predicate this row already keys `data-silent` on — a Play
                  button over a lane with no voice is a control that can only do
                  nothing, which is the readout-that-lies failure this panel
                  keeps guarding against. */}
              {canSound(entry) && (
                <button
                  type="button"
                  className="kit-lane__play"
                  aria-label={t('kit.playLane', { lane: t(`lanes.${entry.lane}`) })}
                  title={t('kit.playLane', { lane: t(`lanes.${entry.lane}`) })}
                  onClick={() => void auditionLane(entry.lane)}
                >
                  <Play size={12} aria-hidden="true" />
                </button>
              )}

              {/* ⛔ **The dice** (TASK-050A). Per pad, re-rolling it from the
                  folder the browser is showing — filtered by what the filename
                  says the file is, so a crash cannot land on the kick. A locked
                  pad is exempt and the store says so rather than doing nothing
                  quietly. */}
              <button
                type="button"
                className="kit-lane__dice"
                disabled={busy}
                aria-label={t('kit.randomizeOne', { lane: t(`lanes.${entry.lane}`) })}
                title={t('kit.randomizeOne', { lane: t(`lanes.${entry.lane}`) })}
                onClick={() => void randomize(entry.lane)}
              >
                <Dices size={12} aria-hidden="true" />
              </button>

              {/* ⛔⛔ **THE SOUND'S OWN EDITOR** (TASK-164). Mike asked whether
                  the drum lanes had one — *"Kick, Sub Bass, Rim Shot, etc."* —
                  and the answer was that the notes had an editor and the sound
                  did not.

                  ⚠ **On the same `canSound` predicate as the Play button**, and
                  for the same reason: an envelope over a lane with no voice is a
                  control that can only do nothing. It is deliberately *not*
                  gated on `entry.name` — the whole point is that a shipped kick
                  can be shaped too, not only a sample the producer replaced. */}
              {canSound(entry) && (
                <button
                  type="button"
                  className="kit-lane__edit"
                  aria-label={t('kit.editPad', { lane: t(`lanes.${entry.lane}`) })}
                  title={t('kit.editPad', { lane: t(`lanes.${entry.lane}`) })}
                  aria-expanded={editing === entry.lane}
                  onClick={() => setEditing(editing === entry.lane ? null : entry.lane)}
                >
                  <SlidersHorizontal size={12} aria-hidden="true" />
                </button>
              )}

              {entry.name && (
                <button
                  type="button"
                  className="kit-lane__clear"
                  disabled={busy}
                  aria-label={t('kit.clearOne', { lane: t(`lanes.${entry.lane}`) })}
                  title={entry.path ?? undefined}
                  onClick={() => void clear(entry.lane)}
                >
                  <X size={12} aria-hidden="true" />
                </button>
              )}

              {/* ⚠ **Inside the row it belongs to**, so the controls sit under
                  the pad they change rather than in a floating window a producer
                  has to relate back to a lane by name. */}
              {editing === entry.lane && (
                <PadEditor entry={entry} onClose={() => setEditing(null)} />
              )}
            </li>
          );
        })}
      </ul>

      {/* Named kits (TASK-051). Below the lanes, because you build a kit
          and then save it — the order a producer works in. */}
      <p className="kit-hint kit-hint--saved">{t('kits.heading')}</p>
      <SavedKits />

      {error && (
        <p className="kit-error" role="alert">
          {error}
        </p>
      )}

      {/* ⛔ **What the batch did, per file** (TASK-049). `role="status"` rather
          than `role="alert"`: eighteen of twenty landing is a result, not an
          emergency, and a screen reader that interrupts for a working import is
          the same over-reporting the `Cancelled` rule exists to avoid. */}
      {imported && (
        <div className="kit-import" role="status">
          <p className="kit-import__count">{t('kit.imported', { count: imported.loaded })}</p>
          {imported.refused.length > 0 && (
            <>
              <p className="kit-import__heading">{t('kit.notPlaced')}</p>
              <ul className="kit-import__refused">
                {imported.refused.map((one) => (
                  // ⚠ Keyed by name AND reason: a batch may well contain two
                  // files of the same name from two folders, refused for two
                  // different reasons.
                  <li key={`${one.name}:${one.reason}`}>
                    <b>{one.name}</b> — {one.reason}
                  </li>
                ))}
              </ul>
            </>
          )}
          <button type="button" className="btn-ghost" onClick={dismissImport}>
            {t('kit.dismissImport')}
          </button>
        </div>
      )}
    </>
  );
}
