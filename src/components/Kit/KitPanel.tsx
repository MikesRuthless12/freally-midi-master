import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Dices, X } from 'lucide-react';

import { canSound, useKit } from '../../state/kit';
import { SavedKits } from './SavedKits';
import { useExplorer } from '../../state/explorer';
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
const SAMPLE_TYPE = 'application/x-freally-sample';

/** The path a drop is carrying, or `null` when it is not one of ours. */
function droppedSample(transfer: DataTransfer): string | null {
  const path = transfer.getData(SAMPLE_TYPE);
  return path === '' ? null : path;
}

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
  const lanes = useKit((s) => s.lanes);
  const loaded = useKit((s) => s.loaded);
  const assigning = useKit((s) => s.assigning);
  const error = useKit((s) => s.error);
  const refresh = useKit((s) => s.refresh);
  const assign = useKit((s) => s.assign);
  const clear = useKit((s) => s.clear);
  const randomize = useKit((s) => s.randomize);
  const dropOn = useExplorer((s) => s.dropOn);
  // Which row the pointer is currently over, so the target is visible before
  // the producer lets go. Local: it is a property of this gesture, not of the
  // kit, and nothing outside this panel can act on it.
  const [over, setOver] = useState<Lane | null>(null);

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
        disabled={assigning !== null}
        aria-label={t('kit.randomize')}
        title={t('kit.randomize')}
        onClick={() => void randomize(null)}
      >
        <Dices size={14} aria-hidden="true" /> {t('kit.randomize')}
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
                void dropOn(entry.lane, path).then(() => refresh());
              }}
            >
              <button
                type="button"
                className="kit-lane__pad"
                // ⚠ Disabled only while *this* panel has a dialog open, and it
                // has to be: two native dialogs from one plugin is a window a
                // producer cannot explain, and the plugin refuses the second
                // one anyway.
                disabled={assigning !== null}
                onClick={() => void assign(entry.lane)}
              >
                <span className="kit-lane__name">{t(`lanes.${entry.lane}`)}</span>
                <span className="kit-lane__source">{busy ? t('kit.choosing') : source}</span>
              </button>

              {/* ⛔ **The dice** (TASK-050A). Per pad, re-rolling it from the
                  folder the browser is showing — filtered by what the filename
                  says the file is, so a crash cannot land on the kick. A locked
                  pad is exempt and the store says so rather than doing nothing
                  quietly. */}
              <button
                type="button"
                className="kit-lane__dice"
                disabled={assigning !== null}
                aria-label={t('kit.randomizeOne', { lane: t(`lanes.${entry.lane}`) })}
                title={t('kit.randomizeOne', { lane: t(`lanes.${entry.lane}`) })}
                onClick={() => void randomize(entry.lane)}
              >
                <Dices size={12} aria-hidden="true" />
              </button>

              {entry.name && (
                <button
                  type="button"
                  className="kit-lane__clear"
                  disabled={assigning !== null}
                  aria-label={t('kit.clearOne', { lane: t(`lanes.${entry.lane}`) })}
                  title={entry.path ?? undefined}
                  onClick={() => void clear(entry.lane)}
                >
                  <X size={12} aria-hidden="true" />
                </button>
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
    </>
  );
}
