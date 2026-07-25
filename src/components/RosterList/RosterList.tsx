import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
import './RosterList.css';

/**
 * Everyone the app can generate from, browsable without typing (FR-009).
 *
 * Artists first, then genres. That order is the product's claim: "Trap is not
 * Metro Boomin" — the named artist is what someone came for, and the genre is
 * the fallback underneath.
 */
export function RosterList() {
  const { t } = useTranslation();
  const roster = useSession((s) => s.roster);
  const selectedId = useSession((s) => s.selectedId);
  const select = useSession((s) => s.select);

  const artists = roster.filter((entry) => entry.type === 'artist');
  const genres = roster.filter((entry) => entry.type === 'genre');

  const group = (label: string, entries: typeof roster) =>
    entries.length > 0 && (
      <li className="roster__group" key={label}>
        <h3 className="roster__heading">{label}</h3>
        <ul className="roster__items">
          {entries.map((entry) => (
            <li key={entry.id}>
              <button
                type="button"
                className={`roster__item${selectedId === entry.id ? ' roster__item--selected' : ''}`}
                aria-pressed={selectedId === entry.id}
                onClick={() => select(entry.id)}
              >
                <span className="roster__name">{entry.name}</span>
                {entry.tier === 'flagship' && (
                  <span className="badge badge--flagship">{t('roster.flagship')}</span>
                )}
              </button>
            </li>
          ))}
        </ul>
      </li>
    );

  return (
    <ul className="roster" aria-label={t('sections.roster')}>
      {group(t('roster.artists'), artists)}
      {/* The section heading, not a second copy of the same word — the left
          rail already has "Genres" translated in every catalog. */}
      {group(t('sections.genres'), genres)}
    </ul>
  );
}
