import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
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
    tendsTo.push(t('artist.tempo', { bpm: Math.round(defaults.bpm) }));
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
    if ((defaults.moods ?? []).length > 0) {
      tendsTo.push(t('artist.moods', { count: (defaults.moods ?? []).length }));
    }
  }

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
    </section>
  );
}
