import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { X } from 'lucide-react';

import { canSound, useKit } from '../../state/kit';

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

      {error && (
        <p className="kit-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
