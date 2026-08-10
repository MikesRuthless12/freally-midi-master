import { Pencil } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
import type { RosterEntry } from '../../lib/ipc-types';
import './RosterList.css';

/**
 * Everyone the app can generate from, browsable without typing (FR-009).
 *
 * Artists first, then genres. That order is the product's claim: "Trap is not
 * Metro Boomin" — the named artist is what someone came for, and the genre is
 * the fallback underneath.
 *
 * The lists arrive as props rather than being read from the store, because the
 * left rail cross-filters them against the current selection and its genre
 * chips have to agree with this list exactly. One place decides; this draws it.
 */
export function RosterList({
  artists,
  genres,
  onEdit,
}: {
  artists: RosterEntry[];
  genres: RosterEntry[];
  /**
   * Open a style of the producer's own in the editor.
   *
   * ⛔ **Without this a saved style could never be edited or deleted.** The
   * editor's delete path and its read-back-what-was-saved path both key on an
   * id, and nothing in the app ever passed one — so a producer accumulated user
   * models with no way to change or remove any of them. The badge said the row
   * was theirs and the row did nothing about it.
   */
  onEdit: (id: string) => void;
}) {
  const { t } = useTranslation();
  const selectedId = useSession((s) => s.selectedId);
  const select = useSession((s) => s.select);

  const group = (label: string, entries: RosterEntry[]) =>
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
                {/* ⛔ **A GENRE badge as well as the flagship one** (TASK-047).
                    It earns its place in *search results* rather than in this
                    list: there the two kinds are mixed together with no heading
                    above them, so the row is the only thing that can say
                    whether "Trap" is an artist or the archetype under one. */}
                {/* ⛔ **A style of the producer's own says so first** (TASK-040U).
                    It beats the tier badge rather than sitting beside it: a user
                    model inherits its parent's `tier`, so a style built on Drake
                    would otherwise wear FLAGSHIP — claiming a provenance it does
                    not have, in the one row that exists to say what a thing is. */}
                {entry.mine ? (
                  <span className="badge badge--mine">{t('styles.mine')}</span>
                ) : entry.type === 'genre' ? (
                  <span className="badge badge--genre">{t('roster.genre')}</span>
                ) : (
                  entry.tier === 'flagship' && (
                    <span className="badge badge--flagship">{t('roster.flagship')}</span>
                  )
                )}
              </button>

              {/* ⚠ Its own control rather than a click-through on the row: the
                  row selects, and selecting a style to generate from is not the
                  same gesture as opening it to change. */}
              {entry.mine && (
                <button
                  type="button"
                  className="roster__edit"
                  aria-label={t('styles.edit', { name: entry.name })}
                  title={t('styles.edit', { name: entry.name })}
                  onClick={() => onEdit(entry.id)}
                >
                  <Pencil size={12} aria-hidden="true" />
                </button>
              )}
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
