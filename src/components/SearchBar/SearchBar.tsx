import { useMemo, useRef, useState } from 'react';
import { Search } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { search } from '../../lib/fuzzy';
import { useSession } from '../../state/session';
import './SearchBar.css';

/**
 * Artist and genre search with an autosuggest list (FR-009, US-001).
 *
 * The whole thing runs against the roster already in memory, so a keystroke
 * costs no IPC. Keyboard first: ↑↓ move, Enter selects, Esc closes — a producer
 * typing a name should never have to reach for the mouse.
 *
 * Implemented as a combobox rather than a plain input with a div under it, so
 * a screen reader announces the count and the highlighted option. Getting that
 * wrong makes the feature invisible rather than merely awkward.
 */
export function SearchBar() {
  const { t } = useTranslation();
  const roster = useSession((s) => s.roster);
  const rosterLoaded = useSession((s) => s.rosterLoaded);
  const select = useSession((s) => s.select);

  const [query, setQuery] = useState('');
  const [open, setOpen] = useState(false);
  const [activeRow, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const results = useMemo(() => search(query, roster), [query, roster]);

  // Clamped rather than reset in an effect: a shrinking result list must not
  // leave the highlight past the end, and doing it here means the render that
  // shortens the list already draws the right row — no second pass, and no
  // frame where `aria-activedescendant` points at an option that is gone.
  const active = Math.min(activeRow, Math.max(0, results.length - 1));

  const choose = (id: string) => {
    select(id);
    const entry = roster.find((e) => e.id === id);
    setQuery(entry ? entry.name : '');
    setOpen(false);
    inputRef.current?.blur();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape') {
      setOpen(false);
      return;
    }
    if (!open || results.length === 0) {
      // Down on a closed box opens it, which is what a combobox is expected to
      // do and costs nothing.
      if (event.key === 'ArrowDown' && results.length > 0) setOpen(true);
      return;
    }

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActive((i) => (i + 1) % results.length);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActive((i) => (i - 1 + results.length) % results.length);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      choose(results[active].id);
    }
  };

  const showList = open && query.trim() !== '';

  return (
    <div className="search-bar">
      <div className="search">
        <Search className="search__icon" size={16} aria-hidden="true" />
        <input
          ref={inputRef}
          className="search__input"
          type="search"
          role="combobox"
          aria-expanded={showList}
          aria-controls="search-results"
          aria-autocomplete="list"
          aria-activedescendant={
            showList && results.length > 0 ? `search-result-${active}` : undefined
          }
          placeholder={t('rails.searchPlaceholder')}
          aria-label={t('rails.searchLabel')}
          value={query}
          disabled={!rosterLoaded}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          // A click on an option would otherwise be lost to the blur that
          // precedes it, so closing is deferred past the mousedown.
          onBlur={() => window.setTimeout(() => setOpen(false), 120)}
          onKeyDown={onKeyDown}
        />
      </div>

      {showList && (
        <ul
          className="suggest"
          id="search-results"
          role="listbox"
          aria-label={t('rails.results')}
        >
          {results.length === 0 ? (
            <li className="suggest__empty" role="presentation">
              {t('rails.noMatch', { query })}
            </li>
          ) : (
            results.map((entry, index) => (
              <li
                key={entry.id}
                id={`search-result-${index}`}
                role="option"
                aria-selected={index === active}
                className={`suggest__item${index === active ? ' suggest__item--active' : ''}`}
                onMouseEnter={() => setActive(index)}
                onMouseDown={(event) => {
                  // Keep focus so the blur handler does not close the list out
                  // from under the click.
                  event.preventDefault();
                  choose(entry.id);
                }}
              >
                <span className="suggest__name">{entry.name}</span>
                {entry.tier === 'flagship' && (
                  <span className="badge badge--flagship">{t('roster.flagship')}</span>
                )}
                {entry.type === 'genre' && (
                  <span className="badge badge--genre">{t('roster.genre')}</span>
                )}
                {entry.era && <span className="suggest__era">{entry.era}</span>}
              </li>
            ))
          )}
        </ul>
      )}
    </div>
  );
}
