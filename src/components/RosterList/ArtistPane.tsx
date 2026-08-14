import { useTranslation } from 'react-i18next';

import { GENERATED_PARTS, useSession } from '../../state/session';
import type { RosterEntry } from '../../lib/ipc-types';
import './ArtistPane.css';

/**
 * What the selected artist or genre actually is (TASK-047, FR-009).
 *
 * ⛔ **The point is that browsing a roster of 500 should not require pressing
 * Generate to find out what something sounds like.** A name and a tier badge
 * say almost nothing; era, the genres it works in, and what it *tends to do*
 * are what let a producer choose. The roadmap calls it the "tends to" summary
 * and asks for it "from the resolved model" — which is what `session_defaults`
 * already answers on selection, before any generation.
 *
 * ⛔ **Read from `defaults`, never from the pins.** `defaults` is what the
 * *artist* asks for; the pins are what the producer has overridden. A pane that
 * showed the pins would say "F♯ minor at 140" about an artist who plays neither
 * — which is the readout-that-lies failure the session chips were built to
 * avoid, one panel over.
 */
export function ArtistPane({ entry }: { entry: RosterEntry | null }) {
  const { t } = useTranslation();
  const defaults = useSession((s) => s.defaults);

  // Nothing selected is not an empty pane — it is no pane. A heading over
  // blanks reads as something that failed to load.
  if (entry === null) return null;

  const genre = entry.type === 'genre';
  const tendsTo: string[] = [];
  if (defaults !== null) {
    // ⛔⛔ **The range, not just the nominal** (TASK-158D — Mike asks for "tempo
    // range" by name). An artist who works at 68–96 and one who works at 138–142
    // are different propositions at the same nominal 82, and choosing between
    // them is what this pane is for.
    //
    // ⚠ **Collapsed to one number when the model authored no range**, because
    // `SessionDefaults` answers the nominal twice in that case — "82 – 82 BPM"
    // would read as a bound the model never stated.
    const low = Math.round(defaults.bpmMin);
    const high = Math.round(defaults.bpmMax);
    tendsTo.push(
      low === high
        ? t('artist.tempo', { bpm: Math.round(defaults.bpm) })
        : t('artist.tempoRange', { low, high }),
    );
    // ⚠ The **first** key and scale, not the whole list: these are what the
    // model draws from in authored order, and a pane listing six of each would
    // be a data dump rather than a summary. The chips are where the full choice
    // lives.
    if (defaults.keys.length > 0) {
      tendsTo.push(
        t('artist.key', {
          key: defaults.keys[0],
          scale: t(`scales.${defaults.scales[0]}`, defaults.scales[0] ?? ''),
        }),
      );
    }
    if (defaults.halfTime) tendsTo.push(t('artist.halfTime'));
  }

  /**
   * The moods, by name (TASK-158D).
   *
   * ⛔ **Named rather than counted.** This said "4 moods", which tells a producer
   * there is something to pick and nothing about whether they want it — and the
   * mood chip is one panel over, so the count was a pointer to a control rather
   * than a description of the artist.
   */
  const moods = defaults?.moods ?? [];

  /**
   * What this model writes, and what it does not (TASK-158D).
   *
   * ⛔⛔ **The half that was missing, and it is the one that prevents silence.**
   * `bass.rs`, `chords.rs`, `melody.rs` and `counter.rs` each return an **empty**
   * track when the model authored no block of their own — correct behaviour, an
   * artist who does not write melodies should not have one invented for them —
   * so pressing Generate on the Melody tab for such an artist produces nothing at
   * all, and until now the only way to learn that was to press it.
   *
   * ⚠ **`engine::context::parts_of` decides, not this component.** The rules are
   * subtle in two places — drums always generate, and a `bass808` whose role is
   * `bassline` means the sub comes out of the kit rather than the bass generator
   * — and both are facts about the engine. `engine/tests/coverage.rs` checks every
   * shipped model by *generating* it, so this list cannot drift into a promise the
   * engine does not keep.
   *
   * ⛔⛔ **BOTH LISTS ARE EMPTY UNTIL `defaults` ARRIVES, and that is not a
   * detail.** `session.ts` sets `defaults: null` in three places — the initial
   * store, every artist switch *before* the fetch, and when `session_defaults`
   * fails outright. Read unguarded, `covers` is `[]` and `missing` is all five,
   * so the pane announced **"Does not write: Drums · Chords · Melody · Counter ·
   * Bass"** for the length of every IPC round trip, and permanently on a failed
   * one — about a model that certainly writes drums, since `parts_of` documents
   * them as always covered. That is the readout-that-lies failure arriving
   * through the feature built to close it. `null` means *not known yet*, and the
   * honest rendering of not-known-yet is nothing at all.
   */
  const covers = defaults?.parts ?? [];
  // ⚠ **`GENERATED_PARTS`, not a list of this component's own.** It is already
  // the five in `engine::pattern::PART_ORDER`'s sequence — and a local copy
  // annotated `Part[]` is exactly the vacuous form `SongTimeline/clips.ts` warns
  // about: a sixth part would drop out of this line in silence.
  const missing = defaults === null ? [] : GENERATED_PARTS.filter((p) => !covers.includes(p));

  return (
    <section className="artistpane" aria-label={t('artist.label')}>
      <h4 className="artistpane__name">
        {entry.name}
        {/* ⛔ **The GENRE badge is the pair to the flagship one**, and it earns
            its place in *search results* rather than in the list: there the two
            kinds are mixed together with no headings above them, so a row is
            the only thing that can say whether "Trap" is an artist or the
            archetype under one. */}
        <span className={`badge badge--${genre ? 'genre' : (entry.tier ?? 'standard')}`}>
          {genre ? t('roster.genre') : t(`roster.${entry.tier ?? 'standard'}`)}
        </span>
      </h4>

      {entry.era !== null && <p className="artistpane__era">{entry.era}</p>}

      {entry.genres.length > 0 && (
        <ul className="artistpane__tags">
          {entry.genres.map((tag) => (
            <li key={tag} className="artistpane__tag">
              {tag}
            </li>
          ))}
        </ul>
      )}

      {tendsTo.length > 0 && (
        <p className="artistpane__tends">
          {/* ⚠ Joined with a middle dot rather than commas, so it reads as a
              set of facts rather than as a sentence a translator would have to
              rebuild for word order. */}
          <span className="artistpane__label">{t('artist.tendsTo')}</span> {tendsTo.join(' · ')}
        </p>
      )}

      {moods.length > 0 && (
        <p className="artistpane__tends">
          <span className="artistpane__label">{t('artist.moods')}</span> {moods.join(' · ')}
        </p>
      )}

      {/* ⛔ **Both halves, and the second is the one that matters.** "Writes
          drums, chords and melody" is useful; "does not write a bassline" is what
          stops a producer pressing Generate on the Bass tab and concluding the
          app is broken. `missing` is empty for a complete model, so the line
          simply does not appear. */}
      {covers.length > 0 && (
        <p className="artistpane__tends">
          <span className="artistpane__label">{t('artist.writes')}</span>{' '}
          {covers.map((part) => t(`tabs.${part}`)).join(' · ')}
        </p>
      )}
      {missing.length > 0 && (
        <p className="artistpane__tends artistpane__missing">
          <span className="artistpane__label">{t('artist.doesNotWrite')}</span>{' '}
          {missing.map((part) => t(`tabs.${part}`)).join(' · ')}
        </p>
      )}
    </section>
  );
}
