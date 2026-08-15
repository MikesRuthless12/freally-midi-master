import { useState } from 'react';
import { Pencil } from 'lucide-react';
import { Section } from './Section';
import { StyleEditor } from '../StyleEditor/StyleEditor';
import '../StyleEditor/StyleEditor.css';
import { RailResizer } from './RailResizer';
import { RailTabs } from './RailTabs';
import { SWAP_STYLE, useUi } from '../../state/ui';
import { Combo } from '../Combo/Combo';
import { ExplorerPanel } from '../Explorer/ExplorerPanel';
import { ArtistPane } from '../RosterList/ArtistPane';
import { crossFilter } from '../../lib/cross-filter';
import { DECADES, matchesEras } from '../../lib/era';
import { search } from '../../lib/fuzzy';
import type { RosterEntry } from '../../lib/ipc-types';
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

  // `null` is closed; a string is the id of the style being edited, and the
  // empty string is a new one. Three states rather than two booleans, because
  // "open" and "which" cannot disagree if there is only one of them.
  //
  // ⚠ **`showAllFor` went with the notice that owned it** (2026-08-11). It held
  // the one id the cross-filter was to be suspended for, cleared whenever the
  // selection moved on — machinery for a "Show all" button that no longer
  // exists. With nothing able to set it, the filter is simply the selection.
  const [editing, setEditing] = useState<string | null>(null);
  const eras = useUi((s) => s.eras);
  const toggleEra = useUi((s) => s.toggleEra);
  const { artists, filteredBy } = crossFilter(roster, selectedId);

  const selected = roster.find((entry) => entry.id === selectedId) ?? null;

  // ⛔⛔ **A QUERY reaches the whole roster; the OPEN LIST follows the genre.**
  // Mike, 2026-08-12: *"changing to another genre doesn't change the actual
  // names in the Roster to names within that Genre, it keeps the same names and
  // then shows different genres within the Roster list."* Picking a genre and
  // watching the roster answer with the same ten names is the genre box visibly
  // doing nothing.
  //
  // ⛔ **The rule this replaces — "never cross-filter a combobox" — was drawn
  // wider than the defect that taught it.** That defect was the rail
  // auto-selecting an artist on load: `crossFilter` trimmed the genres to that
  // artist's own and "UK Drill" could not be typed at all. The auto-select is
  // gone, and the part worth keeping is narrower than the rule was: what must
  // never be hidden is what a *query* can find. So these two pools stay whole
  // and feed `filter` below, and the browse list is the half that narrows.
  const allArtists = roster.filter((entry) => entry.type !== 'genre');
  const allGenres = roster.filter((entry) => entry.type === 'genre');

  // One row shape for both halves, so a name found by typing draws exactly as
  // it does in the browse list — the two lists are the same control.
  const option = (entry: RosterEntry) => ({
    id: entry.id,
    name: entry.name,
    badge: entry.mine
      ? t('styles.mine')
      : entry.type === 'genre'
        ? t('roster.genre')
        : entry.tier === 'flagship'
          ? t('roster.flagship')
          : null,
  });

  /**
   * The browse list: the way in, then Artists A–Z, then Producers A–Z.
   *
   * ⛔⛔ **Mike, 2026-08-12**: *"put 'Original Workflow' then 'Artists'
   * underlined and then list all artists in alphabetical order and then
   * 'Producers' underlined and then put producers in alphabetical order after
   * that … and can you do this for all genres?"*
   *
   * ⛔ **Sorted here rather than trusted from the loader.** `dataset::roster()`
   * appends the producer's own styles after the shipped ones deliberately — the
   * rail decides where those appear, not the loader — so the incoming order is
   * "shipped, then yours", which is not alphabetical in either group.
   *
   * ⚠ **A heading only when its group has anyone in it.** Nine of the shipped
   * genres are producer-only and several are artist-only; an "Artists" rule with
   * nothing under it reads as a list that failed to load rather than as a group
   * that is empty.
   *
   * ⚠ `localeCompare` rather than `<`: the roster has names like "Pi'erre
   * Bourne" and "Ma$e", and a codepoint sort files the punctuation ahead of
   * every letter.
   */
  const GROUPS = [
    // ⛔ Double underscores for the same reason `ORIGINAL` has them: outside the
    // slug alphabet `presets::is_safe_stem` accepts, so a separator's id cannot
    // collide with a real style even by accident.
    { id: '__artists__', label: t('roster.artists'), holds: 'artist' },
    { id: '__producers__', label: t('roster.producers'), holds: 'producer' },
  ] as const;

  // ⛔⛔ **The era pills narrow the BROWSE list and nothing else** (TASK-158G).
  // Exactly the rule the genre cross-filter follows fifty lines up, for exactly
  // the reason given there: narrowing what you *browse* is a convenience,
  // narrowing what you can *find* is a defect. `filter` below is built from the
  // whole pools, so a producer with the 90s pressed can still type "Yeat" and
  // get him — the pills cannot make a name untypable.
  //
  // ⚠ **Built once per render, not once per reader.** The options list and the
  // "nobody in those years" hint both need it, and computing it twice meant a
  // second `matchesEras` pass plus two more `localeCompare` sorts over the whole
  // roster — measured at ~0.55 ms per render on the 590 that ship.
  const browse = GROUPS.flatMap(({ id, label, holds }) => {
    const list = artists
      .filter((entry) => entry.type === holds && matchesEras(entry.era, eras))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map(option);
    return list.length === 0 ? [] : [{ id, name: label, heading: true }, ...list];
  });

  // ⛔⛔ **The separator WORDS are not reserved, and a version of this that
  // reserved them was wrong** (Mike, 2026-08-12: *"no i don't want it to select
  // the word 'Artists' or the word 'Producers' not the actual artist/producer
  // themselves"*). What may never be selected is the **row** — the label that
  // resolves to no style — and that is structural: a heading is not a
  // `role="option"`, `step` walks over it, `commit` skips it and the mouse has
  // no handler on it. Typing "Artists" and landing on a real name the matcher
  // considers close is the search doing its job, and blocking it would take a
  // way in away for no defect.

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
    <aside className="rail rail--left" style={SWAP_STYLE}>
      {/* ⛔ **Above the search box and every section, always** (TASK-040U).
          Mike's rule is that this pins to the top of the roster above every
          artist and producer, whoever is selected and whatever the search
          says — it is the way in to building your own, so it cannot be
          something you scroll to find. It sits outside the roster list rather
          than inside it for exactly that reason: no alias, tier or filter can
          reach it. */}
      <div className="rail__panels">
        {/* ⚠ **Outside the slots, so it is not one of the two.** It is a button,
            not a panel — giving it a tab would mean a producer could close the
            way in to building their own style, and losing half the rail to it
            would be worse still. `rail__pinned` keeps it at its own height above
            whatever the slots hold. */}
        <div className="rail__pinned">
          <button type="button" className="rail__original" onClick={() => setEditing('')}>
            {t('styles.original')}
            <small>{t('styles.originalHint')}</small>
          </button>

          {/* ⛔⛔ **Four era pills, above BOTH lists they narrow** (TASK-158G).
              Mike, 2026-08-10: *"allow the end user to [filter] the list by what
              genre/artist was out within those specific years instead of …
              randomly searching for names through genres/artists/producers
              blindly."* A combobox narrowed by typing is fine at thirty names and
              useless at six hundred if you cannot already name the one you want —
              and "what was out when I was listening" is the one axis a producer
              always knows.

              ⛔ **Pinned rather than inside the roster panel, because it governs
              the genre box too.** A control that sits in one collapsible section
              and silently narrows another is a control whose reach you cannot
              see; and the sections collapse, so a pill left pressed inside a
              closed panel would filter both lists with nothing on screen to say
              why. Up here it is always visible, exactly like the way in above it.

              ⛔ **Multi-select, and a model matches on OVERLAP.** `boom-bap` is
              `1990s–present`, so it belongs under all four at once; a sort would
              force it into one bucket and lie about three. `lib/era.ts` owns that
              comparison.

              ⚠ **The decades are digits, so they carry no catalog key.** "2010s"
              is the same four characters in every locale this ships in; only the
              group needs a name, which is what `roster.era` is.

              ⚠ Buttons in a `group` rather than checkboxes: each is a toggle
              whose pressed state is `aria-pressed`, which is what a pill is. */}
          <div
            className="rail__eras"
            role="group"
            aria-label={t('roster.era')}
            data-testid="era-pills"
          >
            {DECADES.map((decade) => (
              <button
                key={decade}
                type="button"
                className="btn-ghost rail__era"
                aria-pressed={eras.includes(decade)}
                onClick={() => toggleEra(decade)}
              >
                {decade}s
              </button>
            ))}
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
            // ⛔ **The era pills reach this list too** (TASK-158G). Mike's
            // sentence is *"what genre/artist was out within those specific
            // years"*, and the pills' own note argues its overlap rule with
            // `boom-bap` — a **genre**, `1990s–present`. A filter that narrowed
            // the artists and left the genres whole would be arguing with a case
            // it did not cover.
            // ⚠ `filter` below is untouched, and that is the same line the
            // roster box draws: narrowing what you browse is a convenience,
            // narrowing what you can find is a defect.
            options={allGenres
              .filter((genre) => matchesEras(genre.era, eras))
              .map((genre) => ({ id: genre.id, name: genre.name }))}
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

        <Section id="roster">
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
                // ⛔ **Artists AND genres are still both *findable* — by typing.**
                // The list this replaced showed both under two headings and the
                // search box found both, so a producer must never have to know
                // which kind "UK Drill" is before they can reach it. That is
                // `filter`'s job below. Listing all fifty-odd genres under the
                // artists as well is what made the browse list unreadable, and
                // is half of what Mike reported.
                // ⛔⛔ **The way in, then the names, and nothing else** — Mike,
                // 2026-08-12: *"if there is no artist/producer then it should
                // just have 'Original Workflow' or only 1 artist like ny-drill
                // /uk-drill then it should just say 'Original Workflow' & 'Pop
                // Smoke'."* Not even the selected genre gets a row of its own:
                // the box above already says which genre it is, and repeating it
                // here would put a genre back in a list whose whole job is to
                // answer "who works in this".
                //
                // ⚠ **A genre is still *choosable* here, it just is not
                // *listed*** — typing "UK Drill" into this box selects it, which
                // is what keeps a producer from having to know which kind a name
                // is. `valueLabel` is what lets the field say so; see below.
                options={[
                  { id: ORIGINAL, name: t('styles.original'), action: true },
                  // ⚠ `crossFilter`'s artists: the whole list when an artist or
                  // nothing is selected, and the genre's own when a genre is —
                  // then split into the two named groups and sorted inside each,
                  // and narrowed by whichever era pills are pressed.
                  ...browse,
                ]}
                value={selectedId}
                // ⛔⛔ **Or the box empties the moment a genre is chosen in it.**
                // `Combo` reads its text out of `options`, and genres no longer
                // have a row — so typing "UK Drill" and pressing Enter selected
                // UK Drill and left the field blank, discarding on screen exactly
                // what the producer had just typed. Caught by
                // `magic-moment.spec.ts`, which arrows to the last suggestion and
                // asserts the box says what it took.
                valueLabel={selected?.type === 'genre' ? selected.name : null}
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
                //
                // ⛔⛔ **Over the whole roster, however narrow the list above
                // is.** Built from the pools rather than looked up in `options`,
                // which is the difference that matters: a lookup would have
                // silently dropped every result the genre had filtered out, so
                // choosing Afroswing would have made Drake untypable. Narrowing
                // what you *browse* is a convenience; narrowing what you can
                // *find* is the defect the old no-cross-filtering rule existed
                // to prevent, and it still is.
                filter={(query) => {
                  // ⚠ **No separators in here, and that is the whole guard.**
                  // The results are built from the roster pools, which hold only
                  // real models — so a typed query cannot surface a heading even
                  // though the browse list above is full of them.
                  // ⚠ Searched over both halves together, in one ranking — two
                  // separate searches concatenated would put every artist above
                  // every genre regardless of how well either matched.
                  const pool = [...allArtists, ...allGenres];
                  return search(query, pool, pool.length).map(option);
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

              {/* ⛔⛔ **"Filtered by … / Show all" IS GONE, BY INSTRUCTION**
                (Mike, 2026-08-11): *"I also don't want the 'Filtered by
                DrakeShow all' to show up at all in the details part of the
                roster."* It ran the two strings together on screen because they
                were a notice and a button sitting side by side under the artist
                blurb — but the real problem is that it had **nothing left to
                describe**. It narrowed the roster *list*, and the list is gone:
                the comboboxes deliberately offer the whole roster (see the note
                on `allArtists` above), so the notice announced a filter that no
                longer applied to any control on screen.

                ⚠ **`noneInGenre` stays**, and it is not the same thing: it
                answers "this genre has no artists in it", which is a fact about
                the selection rather than a filter the producer can undo. */}
              {filteredBy?.type === 'genre' && artists.length === 0 && (
                <p className="rail__hint">
                  {t('roster.noneInGenre', { name: filteredBy.name })}
                </p>
              )}

              {/* ⛔ **Said, rather than an empty list nobody can explain.** With
                a narrow genre selected and one pill pressed the browse list can
                come back with nobody in it — and a combobox that opens onto
                "Original Workflow" alone looks like a roster that failed to load
                rather than a filter doing its job.
                ⚠ **`artists.length > 0` is what keeps this off the other empty
                case.** `noneInGenre` above answers "this genre has nobody in it";
                this answers "the pills left nobody", and showing both at once
                would say the same thing twice in different words. With no pill
                pressed `browse` is empty only when `artists` is, so this cannot
                fire on its own. */}
              {artists.length > 0 && browse.length === 0 && (
                <p className="rail__hint">{t('roster.noneInEra')}</p>
              )}
            </>
          ) : (
            <p className="rail__hint">{t('rails.noDataset')}</p>
          )}
        </Section>

        {/* The sample browser and its audition player (TASK-132). After the
            roster on the strip, because it is where a producer goes *after*
            choosing an artist — and it is the one of the three that starts
            closed (Mike, 2026-08-11: *"file explorer should be hidden for the
            left hand side"*), because opening the browser is a decision rather
            than a state the app should boot into. */}
        <Section id="explorer">
          <ExplorerPanel />
        </Section>
      </div>

      <RailTabs rail="left" />

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
