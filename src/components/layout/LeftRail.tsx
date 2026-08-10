import { useState } from 'react';
import { Pencil } from 'lucide-react';
import { Section } from './Section';
import { StyleEditor } from '../StyleEditor/StyleEditor';
import '../StyleEditor/StyleEditor.css';
import { RailResizer } from './RailResizer';
import { Combo } from '../Combo/Combo';
import { ExplorerPanel } from '../Explorer/ExplorerPanel';
import { ArtistPane } from '../RosterList/ArtistPane';
import { crossFilter } from '../../lib/cross-filter';
import { search } from '../../lib/fuzzy';
import { useSession } from '../../state/session';
import { useTranslation } from 'react-i18next';

/*
 * ⚠ **`GENRE_CHIPS` is gone with the chips it named** (2026-08-09). It held six
 * quick-pick ids because a chip row can only show a handful; the combobox lists
 * every genre and finds any of them by typing, so a shortlist would now be a rule
 * about which genres are worth reaching quickly — a decision nobody made and one
 * the dataset would outgrow silently.
 */

/**
 * The roster combobox's first row: start a style of your own.
 *
 * ⛔ Not a style id — nothing resolves it, and it must never reach the session.
 * The double underscores make that obvious at a glance and keep it outside the
 * slug alphabet `presets::is_safe_stem` accepts, so it cannot collide with a
 * real one even by accident.
 */
const ORIGINAL = '__original__';

/**
 * Left rail: build-your-own on top, then the genre and roster comboboxes.
 *
 * ⛔ **One control per thing, not three.** This was a search box, a chip row and
 * a five-hundred-row list, with the artist's description below all of it — so
 * reading about the artist you had just picked meant scrolling away from the
 * list you picked them from. Mike, 2026-08-09: *"instead of listing the roster,
 * can we just have a combobox … it shows the details under it"* and *"we won't
 * need the search textbox … it will save us some room."*
 */
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
  // `null` is closed; a string is the id of the style being edited, and the
  // empty string is a new one. Three states rather than two booleans, because
  // "open" and "which" cannot disagree if there is only one of them.
  const [editing, setEditing] = useState<string | null>(null);
  const [showAllFor, setShowAllFor] = useState<string | null>(null);
  const [lastSelected, setLastSelected] = useState<string | null>(selectedId);
  if (lastSelected !== selectedId) {
    setLastSelected(selectedId);
    if (showAllFor !== null) setShowAllFor(null);
  }
  const { artists, filteredBy } = crossFilter(
    roster,
    selectedId === showAllFor ? null : selectedId,
  );

  const selected = roster.find((entry) => entry.id === selectedId) ?? null;

  // ⛔⛔ **The comboboxes offer the WHOLE roster, never the cross-filtered one.**
  // Narrowing made sense for a list — it was the only way to make five hundred
  // rows scannable — but a combobox is filtered by *typing*, so hiding entries
  // only stops them being found. It broke immediately once the rail auto-selects
  // an artist on load: `crossFilter` then trimmed the genres to that artist's
  // own, and "UK Drill" could not be typed at all.
  //
  // ⚠ `crossFilter` is still used, for the "filtered by" notice and the pane —
  // saying what the selection *implies* is a different job from deciding what a
  // producer is allowed to reach.
  const allArtists = roster.filter((entry) => entry.type !== 'genre');
  const allGenres = roster.filter((entry) => entry.type === 'genre');

  // ⛔⛔ **NOTHING IS SELECTED ON LOAD, AND THAT IS THE POINT.** Mike,
  // 2026-08-10: *"how about we go back to that landing screen, and ensure that
  // they have to pick an artist before they ever even generate anything, because
  // I LOVE that landing screen."*
  //
  // ⚠ **An auto-select lived here for a few hours and was removed.** It came
  // from *"you cannot have an empty field for the comboboxes"* — but selecting
  // an artist to avoid a blank box threw away the whole empty state with it: the
  // "Search an artist. Cook." screen never appeared, Generate was live before
  // anyone had chosen anything, and the session chips never got to ask. That is
  // a great deal more than not showing a blank field.
  //
  // ▶ The blank field is answered by the **placeholder** instead — the combobox
  // shows "Search an artist…" rather than either nothing or a name the app has
  // not actually selected. A placeholder is a prompt; a name would be a lie.
  //
  // ⚠ Either kind is enough to unlock Generate. A genre is a real style model
  // that generates in its own right, so choosing "Trap" is already a complete
  // choice — confirmed by Mike, 2026-08-10.

  return (
    <aside className="rail rail--left">
      {/* ⛔ **Above the search box and every section, always** (TASK-040U).
          Mike's rule is that this pins to the top of the roster above every
          artist and producer, whoever is selected and whatever the search
          says — it is the way in to building your own, so it cannot be
          something you scroll to find. It sits outside the roster list rather
          than inside it for exactly that reason: no alias, tier or filter can
          reach it. */}
      <div className="rail__section">
        <div className="rail__content">
          <button type="button" className="rail__original" onClick={() => setEditing('')}>
            {t('styles.original')}
            <small>{t('styles.originalHint')}</small>
          </button>
        </div>
      </div>

      {/* ⛔⛔ **One combobox instead of a search box and a five-hundred-row
          list** — Mike, 2026-08-09: *"instead of listing the roster, can we just
          have a combobox … and when you end up with an artist/producer in the
          combobox, it shows the details under it?"* and then *"we won't need the
          search textbox … that can go away too and it will save us some room."*
          Typing filters through the same alias-and-typo matcher the search box
          used, so nothing about finding an artist got weaker; what went away is
          two controls doing one job and the scrolling that separated a name from
          its own description. */}
      <Section id="genres">
        <Combo
          label={t('sections.genres')}
          options={allGenres.map((genre) => ({ id: genre.id, name: genre.name }))}
          value={allGenres.some((genre) => genre.id === selectedId) ? selectedId : null}
          onChange={select}
          placeholder={t('sections.genres')}
          // ⛔ **In the matcher's order, not the roster's.** Filtering `options`
          // by a set of ids threw away the ranking and left the list in dataset
          // order — so typing "Trap" put **Boom Bap** at the top and, because the
          // top row is what Enter and blur commit, choosing it. Mapping the
          // results keeps the best match first, which is the whole point of
          // ranking them.
          filter={(query, options) => {
            const byId = new Map(options.map((option) => [option.id, option]));
            return search(query, allGenres, allGenres.length)
              .map((entry) => byId.get(entry.id))
              .filter((option): option is NonNullable<typeof option> => option !== undefined);
          }}
        />
      </Section>

      <Section id="roster" grow>
        {rosterLoaded && roster.length > 0 ? (
          <>
            <Combo
              label={t('sections.roster')}
              // ⛔⛔ **"Original Workflow" is always the first entry** — Mike,
              // 2026-08-09: *"ensure that 'Original Workflow' is at the top of
              // the artist/producer combobox no matter which genre is selected,
              // so that way you can always start an original artist/producer
              // workflow and save it."* The list narrows to the selected genre,
              // so anything *inside* the roster can be filtered away; this is
              // prepended after the filter for exactly that reason. It is the
              // way in to building your own, so no genre may hide it.
              // ⛔ **Artists AND genres, because that is what the roster held.**
              // The list this replaced showed both, under two headings, and the
              // search box found both — so splitting them across two comboboxes
              // quietly meant a producer had to know which kind "UK Drill" was
              // before they could find it. The genre combobox above is a
              // shortcut, not the only door.
              options={[
                { id: ORIGINAL, name: t('styles.original'), action: true },
                ...allArtists.map((artist) => ({
                  id: artist.id,
                  name: artist.name,
                  badge: artist.mine
                    ? t('styles.mine')
                    : artist.tier === 'flagship'
                      ? t('roster.flagship')
                      : null,
                })),
                ...allGenres.map((genre) => ({
                  id: genre.id,
                  name: genre.name,
                  badge: t('roster.genre'),
                })),
              ]}
              value={selectedId}
              // ⚠ It opens the editor rather than selecting anything: there is no
              // style called "Original Workflow" to generate from, and putting a
              // sentinel id into the session would be a selection the plugin
              // could not resolve.
              onChange={(id) => (id === ORIGINAL ? setEditing('') : select(id))}
              placeholder={t('rails.searchPlaceholder')}
              emptyText={(query) => t('rails.noMatch', { query })}
              // ⚠ The roster's own matcher, not a substring test: it knows
              // aliases and tolerates typos, which is most of what made the
              // search box worth having.
              filter={(query, options) => {
                const byId = new Map(options.map((option) => [option.id, option]));
                // ⚠ Searched over both halves together, in one ranking — two
                // separate searches concatenated would put every artist above
                // every genre regardless of how well either matched.
                const pool = [...allArtists, ...allGenres];
                return search(query, pool, pool.length)
                  .map((entry) => byId.get(entry.id))
                  .filter(
                    (option): option is NonNullable<typeof option> => option !== undefined,
                  );
              }}
            />

            {/* ⛔ **Directly under the combobox**, which is the whole point: the
                pane used to sit below a scrolling list, so reading about the
                artist under the pointer meant scrolling away from them. */}
            <ArtistPane entry={roster.find((entry) => entry.id === selectedId) ?? null} />

            {/* A style of the producer's own can be opened from here — the
                pencil lived on the roster row, and the row is gone. */}
            {selected?.mine === true && (
              <button
                type="button"
                className="rail__edit-style"
                onClick={() => setEditing(selected.id)}
              >
                <Pencil size={12} aria-hidden="true" />{' '}
                {t('styles.edit', { name: selected.name })}
              </button>
            )}

            {filteredBy?.type === 'genre' && artists.length === 0 && (
              <p className="rail__hint">{t('roster.noneInGenre', { name: filteredBy.name })}</p>
            )}
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

      {editing !== null && (
        <StyleEditor
          editing={editing === '' ? null : editing}
          onClose={() => setEditing(null)}
        />
      )}
    </aside>
  );
}
