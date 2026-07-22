import { Search } from 'lucide-react';
import { Section } from './Section';
import { useTranslation } from 'react-i18next';

/** Genre chips are a browse filter only — artists are the unit of generation. */
const GENRES = ['Trap', 'Drill', 'Plugg', 'Rage', 'R&B', 'DnB', 'Country', 'Pop'];

/**
 * Left rail: search on top, then collapsible genre and roster panels. Search is
 * genuinely `disabled` until the dataset loads (TASK-016) rather than merely
 * styled that way, so keyboard and screen-reader users are told.
 */
export function LeftRail() {
  const { t } = useTranslation();

  return (
    <aside className="rail rail--left">
      <div className="rail__section">
        <div className="rail__content">
          <div className="search">
            <Search className="search__icon" size={16} aria-hidden="true" />
            <input
              className="search__input"
              type="search"
              placeholder={t('rails.searchPlaceholder')}
              aria-label={t('rails.searchLabel')}
              disabled
            />
          </div>
        </div>
      </div>

      <Section id="genres">
        <div className="chips">
          {GENRES.map((genre) => (
            <button key={genre} type="button" className="chip" disabled>
              {genre}
            </button>
          ))}
        </div>
      </Section>

      <Section id="roster" grow>
        <p className="rail__hint">{t('rails.noDataset')}</p>
      </Section>
    </aside>
  );
}
