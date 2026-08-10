import { useTranslation } from 'react-i18next';
import { ExternalLink, FileMusic, Music, Star } from 'lucide-react';

import { isInside, useExplorer } from '../../state/explorer';

/**
 * Starred samples, one-shots and MIDI files (TASK-058C).
 *
 * ⛔⛔ **Mike, 2026-08-10, and the whole point is the second half of the
 * sentence**: *"we can click just on the name of that 'Starred Favorite' and it
 * will take us to the exact spot of that exact sample in the actual 'File
 * Explorer' browser and if it's not a folder that you still have in the 'File
 * Explorer' then it should take you there in Windows Explorer or the macOS
 * Explorer, so all 'Starred Favorites' should be able to be reachable at any
 * given time."*
 *
 * With only eight folder tabs, a producer *will* close the folder a favourite
 * lives in — so "reachable at any given time" cannot mean "reachable inside this
 * panel". The OS fallback is what makes the promise true rather than usually
 * true, and it is the reason `favourites_reveal` exists at all.
 */
export function Favourites() {
  const { t } = useTranslation();
  const favourites = useExplorer((s) => s.favourites);
  const roots = useExplorer((s) => s.roots);
  const goToFavourite = useExplorer((s) => s.goToFavourite);
  const toggleFavourite = useExplorer((s) => s.toggleFavourite);

  if (favourites.length === 0) return null;

  return (
    <div className="favourites">
      <p className="favourites__title">{t('explorer.favourites')}</p>
      <ul className="favourites__list" aria-label={t('explorer.favourites')}>
        {favourites.map((entry) => {
          // ⚠ Whether this one is still inside an open library folder decides
          // which of the two destinations the click has — and the row says so
          // *before* it is pressed, rather than surprising the producer with a
          // window from another application.
          const inLibrary = roots.some((root) => isInside(entry.path, root.path));

          return (
            <li key={entry.path} className="favourites__row">
              <button
                type="button"
                className="favourites__go"
                title={inLibrary ? entry.path : t('explorer.revealHint', { path: entry.path })}
                onClick={() => void goToFavourite(entry.path)}
              >
                {entry.kind === 'midi' ? (
                  <FileMusic size={11} aria-hidden="true" />
                ) : (
                  <Music size={11} aria-hidden="true" />
                )}
                <span className="favourites__name">{entry.name}</span>
                {/* ⚠ Only on the ones that will leave the app. A badge on every
                    row would say nothing; on these it is the warning. */}
                {!inLibrary && <ExternalLink size={10} aria-hidden="true" />}
              </button>

              {/* ⛔ Mike: *"you can unstar them too and it will delete the memory
                  of what folder they are in for that specific folder."* Reachable
                  from the list as well as from the tree — a favourite whose
                  folder is closed has no row in the tree to unstar it from. */}
              <button
                type="button"
                className="favourites__unstar"
                aria-label={t('explorer.unstar', { name: entry.name })}
                title={t('explorer.unstar', { name: entry.name })}
                onClick={() => void toggleFavourite(entry.path)}
              >
                <Star size={11} aria-hidden="true" fill="currentColor" />
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
