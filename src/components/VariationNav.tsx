import { useState } from 'react';
import { ChevronLeft, ChevronRight, Star } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { isTypingTarget } from '../lib/keyboard';
import { TAB_PART, useSession } from '../state/session';
import { useUi } from '../state/ui';
import { counter, keptKey, madeAt, useVariations } from '../state/variations';
import { TakeHistory } from './TakeHistory';
import './VariationNav.css';

/**
 * The variation history's controls (TASK-045).
 *
 * ◀ and ▶ step one generation through *this part's* log, and what comes back is
 * the whole setup — artist, mood, seed, bars and pins — not just the number.
 *
 * ⛔ **The counter is per part, and that is not tidiness.** Mike: *"it should
 * show the number of generations for each generator separately as totals… '1 /
 * 300' or '5 / 300' melody generations and '1 / 448' or '2 / 448' chord
 * generations."* Rerolling one lane advances that part and nothing else, so a
 * single global number would claim the chords changed when they did not.
 *
 * ⛔ **The arrows are bound *here*, not on `window`.** `←`/`→` nudge notes in
 * the piano roll and walk the sample browser; a global binding would take them
 * from both. The roadmap's note about the horizontal pair is about which keys
 * are free of a *vertical* collision — the scope is what keeps it honest, the
 * same answer `L` got in the drum grid.
 */
export function VariationNav() {
  const { t, i18n } = useTranslation();
  const activeTab = useUi((s) => s.activeTab);
  const part = TAB_PART[activeTab];

  const entries = useVariations((s) => s.entries);
  // ⚠ Subscribed to but read through `counter` — this is what makes the
  // component re-render when a take lands or the cursor moves.
  useVariations((s) => s.position);
  const step = useVariations((s) => s.step);
  const kept = useVariations((s) => s.kept);
  const keep = useVariations((s) => s.keep);
  const recall = useSession((s) => s.recallVariation);
  const generating = useSession((s) => s.generating);
  /**
   * Whether the browsable history is open (TASK-045B).
   *
   * ⚠ **Declared above the early return below**, because a hook after a
   * conditional return is a hook that does not run in every render.
   */
  const [browsing, setBrowsing] = useState(false);

  // Song is an arrangement of the five rather than a part, so it has no log of
  // its own — `SongTimeline` rerolls sections and owns that story.
  if (part === null) return null;

  // ⛔ **Through `counter`, not re-derived here.** The component had its own
  // copy of "which take am I on out of how many", which is the same arithmetic
  // the store already exports — and two copies of a readout is how one of them
  // starts disagreeing. `entries` and `position` are still subscribed to
  // because that is what makes this re-render when a take lands.
  const { position: shown, total } = counter(part);
  const at = shown - 1;
  const here = (entries[part] ?? [])[at] ?? null;
  const isKept = here !== null && kept[keptKey(part, here.seed)] === true;

  const go = (delta: number) => {
    const landed = step(part, delta);
    if (landed !== null) void recall(landed);
  };

  const back = t('variations.back');
  const forward = t('variations.forward');

  return (
    <div
      className="variations"
      role="group"
      aria-label={t('variations.label')}
      onKeyDown={(event) => {
        if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
        if (event.ctrlKey || event.metaKey || event.altKey) return;
        // The arrows move a caret inside a text field, and this group could
        // gain one — the same guard every other handler in the app keeps.
        if (isTypingTarget(event.target)) return;
        event.preventDefault();
        go(event.key === 'ArrowLeft' ? -1 : 1);
      }}
    >
      <button
        type="button"
        className="btn-ghost"
        aria-label={back}
        title={back}
        // ⛔ Disabled at the ends rather than wrapping. Wrapping would take a
        // producer stepping back through a thousand generations to the newest
        // one with nothing saying so.
        disabled={generating || at <= 0}
        onClick={() => go(-1)}
      >
        <ChevronLeft size={14} aria-hidden="true" />
      </button>

      {/* ⛔⛔ **The counter is the way into the browsable history** (TASK-045B).
          Mike: *"so that way you can go through the actual history of all your
          generations and find what you like."* It is a button rather than a new
          control because this is already where a producer is standing when they
          want it — "3 / 40" is the readout you are looking at when you decide you
          want take 12 back, and putting the list anywhere else would mean
          learning a second place for one idea. */}
      <button
        type="button"
        className="variations__count"
        aria-haspopup="dialog"
        aria-expanded={browsing}
        aria-label={t('takes.label')}
        title={t('takes.label')}
        onClick={() => setBrowsing((was) => !was)}
      >
        {total === 0 ? t('variations.none') : `${at + 1} / ${total}`}
      </button>

      {browsing && <TakeHistory onClose={() => setBrowsing(false)} />}

      <button
        type="button"
        className="btn-ghost"
        aria-label={forward}
        title={forward}
        disabled={generating || total === 0 || at >= total - 1}
        onClick={() => go(1)}
      >
        <ChevronRight size={14} aria-hidden="true" />
      </button>

      {here !== null && (
        // ⛔ **Keeping is per take, and it is the input to training**
        // (TASK-040T). It sits with the arrows because this is where a producer
        // is when they decide — auditioning one generation against the last —
        // and a separate panel would mean marking a take somewhere other than
        // where you heard it.
        <button
          type="button"
          className="btn-ghost variations__keep"
          aria-pressed={isKept}
          aria-label={t('styles.keep')}
          title={t('styles.keep')}
          onClick={() => keep(part, here.seed, !isKept)}
        >
          <Star size={14} aria-hidden="true" />
        </button>
      )}

      {here !== null && (
        // ⛔ **The tempo and key that were *used*, with the timestamp.** The
        // pins can be silent about the very tempo that produced these notes —
        // a generation made while the DAW sat at 92 was made at 92 — so showing
        // the pins here would show blank and call it the truth.
        <span className="variations__made" title={madeAt(here.at, i18n.language)}>
          {t('variations.made', {
            bpm: Math.round(here.bpm),
            scale: t(`scales.${here.scale}`, here.scale),
          })}
        </span>
      )}
    </div>
  );
}
