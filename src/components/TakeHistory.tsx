import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Trash2, X } from 'lucide-react';

import { useSession } from '../state/session';
import { madeAt, useVariations } from '../state/variations';
import './TakeHistory.css';

/**
 * Every generation you have made, browsable (TASK-045B).
 *
 * ⛔⛔ **Stepping is not browsing, and Mike said why.** *"if you have generated
 * 20 just 'Trap' and 20 just 'Rage' and 40 just 'Drake' then it should
 * persist"* — and then the sentence that decides the shape: *"so that way you
 * can go through the actual history of all your generations and find what you
 * like."* `VariationNav`'s ◀/▶ answer "back one" and are useless for "find the
 * one I liked out of eighty"; this is the index.
 *
 * ⛔ **Grouped by style, because that is the grouping in his own sentence.** A
 * flat list of 200 takes across four artists is the same problem the arrows
 * have, one screen larger.
 *
 * ⛔ **It reads the PERSISTED history, not this session's log.** The whole point
 * is the takes from the evening you cannot remember the seed of — so the list
 * comes from `takes.json` via `loadHistory`, which spans restarts. This session's
 * generations are in it too: `record` writes both.
 *
 * ⚠ **Choosing a take restores it, which is also how you audition it.** That is
 * the same thing ◀/▶ already do — `recallVariation` regenerates from the seed and
 * arms it — and it is reversible in the sense that matters here, because every
 * take you passed through is still in this list. ⛔ What it is **not** is a
 * preview channel: like the arrows, it replaces the clip on screen, so hand edits
 * to the current take are lost. Auditioning without committing needs a second
 * playback path that does not touch the transport, which is TASK-160's problem
 * rather than this one's, and pretending otherwise here would be the readout
 * lying about what a click does.
 */
export function TakeHistory({ onClose }: { onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const history = useVariations((s) => s.history);
  const loadHistory = useVariations((s) => s.loadHistory);
  const clearHistory = useVariations((s) => s.clearHistory);
  const recall = useSession((s) => s.recallVariation);
  const parkOn = useVariations((s) => s.parkOn);
  const roster = useSession((s) => s.roster);
  const panel = useRef<HTMLDivElement>(null);

  // ⚠ **Read when the panel opens, not at startup.** It is a file read on the
  // host's editor thread, and a producer who never opens this should not pay for
  // it — the same argument `loadFavourites` records about staying out of
  // `refresh`.
  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  // ⚠ Focus moves in on open so Escape and the arrow keys reach the panel rather
  // than whatever the producer last clicked.
  useEffect(() => {
    panel.current?.focus();
  }, []);

  const styles = Object.entries(history)
    .filter(([, takes]) => takes.length > 0)
    // ⛔ **By most recent take, not by name.** A producer opening this is looking
    // for what they were just working on; alphabetical would bury it under
    // whatever they tried once in a previous session.
    //
    // ⚠ **The last element IS the most recent** — `takes.rs` stores each style's
    // list oldest-first, which is what makes "keep the newest" a cap on the right
    // end, and which this component already relies on three lines below. Scanning
    // for the maximum instead would be an O(takes) reduce inside a comparator:
    // roughly sixteen thousand passes at 32 styles × 100 takes, on every render.
    .sort(([, a], [, b]) => (b.at(-1)?.at ?? 0) - (a.at(-1)?.at ?? 0));

  /** A style's own name, or its id when the roster has no row for it. */
  const nameOf = (id: string) => roster.find((entry) => entry.id === id)?.name ?? id;

  return (
    <div
      className="takes"
      role="dialog"
      aria-label={t('takes.label')}
      tabIndex={-1}
      ref={panel}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="takes__bar">
        <h3 className="takes__title">{t('takes.label')}</h3>
        {styles.length > 0 && (
          // ⚠ It is a record of what somebody has been making, so being able to
          // clear it is not a convenience — `recent::clear` carries the same
          // sentence about the other history.
          <button
            type="button"
            className="btn-ghost"
            aria-label={t('takes.clear')}
            title={t('takes.clear')}
            onClick={() => void clearHistory()}
          >
            <Trash2 size={12} aria-hidden="true" />
          </button>
        )}
        <button
          type="button"
          className="btn-ghost"
          aria-label={t('takes.close')}
          title={t('takes.close')}
          onClick={onClose}
        >
          <X size={12} aria-hidden="true" />
        </button>
      </div>

      {styles.length === 0 ? (
        <p className="takes__empty">{t('takes.none')}</p>
      ) : (
        <ul className="takes__styles">
          {styles.map(([id, takes]) => (
            <li key={id}>
              <h4 className="takes__style">
                {nameOf(id)}
                <span className="takes__count">{takes.length}</span>
              </h4>
              <ul className="takes__list">
                {/* ⛔ **Newest first here, oldest first on disk.** The file's
                    order is the order they were made — which is what makes
                    "keep the newest" a cap on the right end — and what a
                    producer wants to see is the last thing they tried. */}
                {[...takes].reverse().map((take) => (
                  <li key={`${take.part}:${take.seed}`}>
                    <button
                      type="button"
                      className="takes__take"
                      onClick={() => {
                        // ⛔ **The cursor moves with the clip.** Without this the
                        // counter went on saying "80 / 80" over the take just
                        // chosen and ◀ jumped back to 79 — the readout and the
                        // grid disagreeing about which take you are on. A no-op
                        // for a take older than this page load; see `parkOn`.
                        parkOn(take.part, take.seed);
                        void recall(take);
                        onClose();
                      }}
                    >
                      {/* ⛔ **Enough to choose between them, which is the whole
                          requirement.** Mike wants to *find what he likes*, and
                          a list of eighty identical rows would be no better than
                          the arrows. The part says which generator, the bars and
                          tempo say what shape it is, and the timestamp is the
                          thing a producer actually remembers — "the one from last
                          night". The seed is the identity and goes in the title,
                          because it is what you would quote and not what you
                          would scan. */}
                      <span className="takes__part">{t(`tabs.${take.part}`)}</span>
                      <span className="takes__detail" title={`#${take.seed}`}>
                        {t('takes.detail', {
                          bars: take.bars,
                          bpm: Math.round(take.bpm),
                        })}
                      </span>
                      <span className="takes__when">{madeAt(take.at, i18n.language)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
