import { Section } from './Section';
import { SearchBar } from '../SearchBar/SearchBar';
import { RosterList } from '../RosterList/RosterList';
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

  const genres = GENRE_CHIPS.map((id) => roster.find((entry) => entry.id === id)).filter(
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
          {genres.map((genre) => (
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
          <RosterList />
        ) : (
          <p className="rail__hint">{t('rails.noDataset')}</p>
        )}
      </Section>
    </aside>
  );
}
