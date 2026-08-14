import { useTranslation } from 'react-i18next';
import { ExternalLink, FileMusic, Music, Trash2 } from 'lucide-react';

import { isInside, useExplorer } from '../../state/explorer';

/**
 * The files the browser opened lately (TASK-058).
 *
 * ⛔⛔ **Mike, 2026-08-12**: *"the history needs to persist with the new versions
 * and so does the file explorer's folder's list."* The folder list already did —
 * `library.json` is written into `%APPDATA%\Freally MIDI Master`, with no version
 * anywhere in the path — but there was no history in the plugin or on the page to
 * make persist. `plugin/src/recent.rs` writes into the same directory so it
 * inherits that property rather than being given its own.
 *
 * ⚠ **Read-only, and reached the same way a favourite is.** A row goes back to
 * the file in the tree when its folder is still open, and out to the OS when it
 * is not — the same two destinations `Favourites` offers, for the same reason:
 * with eight folder tabs a producer *will* have closed the folder something was
 * auditioned from, and a history that cannot take you back is a list of names.
 *
 * ⚠ **Below the stars, not above them.** A favourite is a choice the producer
 * made and a history is a record of what they happened to click; putting the
 * record first would push the choice off a short rail.
 */
export function Recent() {
  const { t } = useTranslation();
  const recent = useExplorer((s) => s.recent);
  const roots = useExplorer((s) => s.roots);
  const goToFavourite = useExplorer((s) => s.goToFavourite);
  const clearRecent = useExplorer((s) => s.clearRecent);

  if (recent.length === 0) return null;

  return (
    <div className="favourites">
      <p className="favourites__title">
        {t('explorer.recent')}
        {/* ⚠ It is a list of where somebody has been, so being able to empty it
            is not a convenience. */}
        <button
          type="button"
          className="favourites__unstar"
          aria-label={t('explorer.clearRecent')}
          title={t('explorer.clearRecent')}
          onClick={() => void clearRecent()}
        >
          <Trash2 size={11} aria-hidden="true" />
        </button>
      </p>
      <ul className="favourites__list" aria-label={t('explorer.recent')}>
        {recent.map((entry) => {
          // Which of the two destinations this row has, said before it is
          // pressed rather than surprising the producer with another window.
          const inLibrary = roots.some((root) => isInside(entry.path, root.path));

          return (
            <li key={entry.path} className="favourites__row">
              <button
                type="button"
                className="favourites__go"
                title={inLibrary ? entry.path : t('explorer.revealHint', { path: entry.path })}
                // ⚠ **`goToFavourite` by name only.** It walks the tree down to a
                // path and falls back to the OS; nothing about it is specific to
                // a star, and a second copy of that walk is the drift the
                // canonicalisation bug in its own comment came from.
                onClick={() => void goToFavourite(entry.path)}
              >
                {entry.kind === 'midi' ? (
                  <FileMusic size={11} aria-hidden="true" />
                ) : (
                  <Music size={11} aria-hidden="true" />
                )}
                <span className="favourites__name">{entry.name}</span>
                {!inLibrary && <ExternalLink size={10} aria-hidden="true" />}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
