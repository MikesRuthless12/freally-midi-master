import { useState } from 'react';
import { Section } from './Section';
import { RailResizer } from './RailResizer';
import { SearchBar } from '../SearchBar/SearchBar';
import { RosterList } from '../RosterList/RosterList';
import { ExplorerPanel } from '../Explorer/ExplorerPanel';
import { crossFilter } from '../../lib/cross-filter';
import { useSession } from '../../state/session';
import { useTranslation } from 'react-i18next';

/**
 * The genres offered as one-click chips.
 *
 * Ids, not display names: the label comes from the model itself, so a renamed
 * genre relabels here rather than drifting into a chip that selects nothing.
 * Every one is a real style model — a genre generates in its own right, it is
 * not merely a tag on the artists under it.
 */
const GENRE_CHIPS = ['trap', 'uk-drill', 'plugg', 'rage', 'rnb-2000s', 'liquid-dnb'];

/** Left rail: search on top, then collapsible genre and roster panels. */
export function LeftRail() {
  const { t } = useTranslation();
  const roster = useSession((s) => s.roster);
  const rosterLoaded = useSession((s) => s.rosterLoaded);
  const selectedId = useSession((s) => s.selectedId);
  const select = useSession((s) => s.select);

  // ⛔ Holds the id **and** is cleared whenever the selection moves away, so
  // "Show all" applies to the press that asked for it and not to every later
  // visit. Holding the id alone made that one entry permanently unfilterable:
  // select it, Show all, select something else, select it again — and the rail
  // silently refused to narrow, with nothing on screen explaining why.
  const [showAllFor, setShowAllFor] = useState<string | null>(null);
  const [lastSelected, setLastSelected] = useState<string | null>(selectedId);
  if (lastSelected !== selectedId) {
    setLastSelected(selectedId);
    if (showAllFor !== null) setShowAllFor(null);
  }
  const { artists, genres, filteredBy } = crossFilter(
    roster,
    selectedId === showAllFor ? null : selectedId,
  );

  // Picking an artist replaces the quick-picks with that artist's own genres —
  // all of them, not the ones that happen to be quick-picks, or an artist who
  // works in none of the six would leave this row empty. Unfiltered, and when a
  // genre is picked, it is the fixed shortcut list it has always been.
  const chips =
    filteredBy?.type === 'artist'
      ? genres
      : GENRE_CHIPS.map((id) => genres.find((entry) => entry.id === id)).filter(
          (entry): entry is NonNullable<typeof entry> => entry !== undefined,
        );

  return (
    <aside className="rail rail--left">
      <div className="rail__section">
        <div className="rail__content">
          <SearchBar />
        </div>
      </div>

      <Section id="genres">
        <div className="chips">
          {chips.map((genre) => (
            <button
              key={genre.id}
              type="button"
              className="chip"
              aria-pressed={selectedId === genre.id}
              onClick={() => select(genre.id)}
            >
              {genre.name}
            </button>
          ))}
        </div>
      </Section>

      <Section id="roster" grow>
        {rosterLoaded && roster.length > 0 ? (
          <>
            {/* Search still reaches every model whatever is filtered here, so
                this is legibility rather than an escape hatch — a narrowed list
                with nothing saying so reads as a dataset that lost half itself. */}
            {filteredBy && (
              <div className="roster__filter">
                <span>{t('roster.filteredBy', { name: filteredBy.name })}</span>
                <button
                  type="button"
                  className="roster__show-all"
                  onClick={() => setShowAllFor(selectedId)}
                >
                  {t('roster.showAll')}
                </button>
              </div>
            )}
            {filteredBy?.type === 'genre' && artists.length === 0 && (
              <p className="rail__hint">{t('roster.noneInGenre', { name: filteredBy.name })}</p>
            )}
            <RosterList artists={artists} genres={genres} />
          </>
        ) : (
          <p className="rail__hint">{t('rails.noDataset')}</p>
        )}
      </Section>

      {/* The sample browser and its audition player (TASK-132). Below the
          roster because it is where a producer goes *after* choosing an artist,
          and because it is the panel worth having the extra width. */}
      <Section id="explorer" grow>
        <ExplorerPanel />
      </Section>

      <RailResizer />
    </aside>
  );
}
