import { Trash2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Combo } from '../Combo/Combo';
import { invoke } from '../../lib/ipc';
import type { Pattern } from '../../lib/ipc-types';
import { useSession } from '../../state/session';
import { useUi } from '../../state/ui';
import './PatternBrowser.css';

/**
 * The pattern library (TASK-045A).
 *
 * The problem it solves, in Mike's words: *"you generate something you like,
 * you have to leave, and when you come back it should still be there — usable
 * with whatever kit you feel like putting under it."*
 *
 * ⛔ **This is not the presets panel and not the project.** A preset is a
 * *starting point* — artist, seed, pins — and stores no notes, because the
 * engine regenerates them. The project stores *this song*, with the host. A
 * saved pattern is the notes themselves, outside any one project, precisely
 * because an edited pattern is not reproducible from its seed and because a
 * producer wants the take they kept rather than the one an engine change would
 * rebuild.
 *
 * ⛔ **No kit is saved with it.** That is what makes "use it with any sounds you
 * want" true rather than a claim: load the pattern, swap the kit, and the same
 * performance plays through different samples.
 */

type PatternSummary = {
  id: string;
  name: string;
  artistId: string;
  part: string;
  bars: number;
  bpm: number;
  savedAt: number;
  /** How busy each sixteenth of the clip is, 0–1 — the mini grid preview. */
  density: number[];
};

/**
 * The mini grid, drawn from the summary rather than from the notes.
 *
 * ⛔ **The whole reason `density` is on the wire.** Drawing a real preview would
 * mean loading every saved pattern's notes to render one row each — hundreds of
 * kilobytes to decide which of them to open. Thirty-two numbers is what the eye
 * needs to tell a sparse boom-bap loop from a busy drill one.
 */
function MiniGrid({ density }: { density: number[] }) {
  return (
    <span className="patterns__preview" aria-hidden="true">
      {density.map((value, index) => (
        <span key={index} className="patterns__step" style={{ opacity: 0.12 + value * 0.88 }} />
      ))}
    </span>
  );
}

export function PatternBrowser() {
  const { t, i18n } = useTranslation();
  const openClip = useSession((s) => s.openClip);
  const patterns = useSession((s) => s.patterns);
  const activeTab = useUi((s) => s.activeTab);

  const [saved, setSaved] = useState<PatternSummary[]>([]);
  const [name, setName] = useState('');
  const [artist, setArtist] = useState('');
  const [part, setPart] = useState('');
  const [error, setError] = useState<string | null>(null);

  const fail = (cause: unknown) =>
    setError(cause instanceof Error ? cause.message : String(cause));

  const refresh = () => {
    invoke<PatternSummary[]>('patterns_list')
      .then((list) => {
        setSaved(list);
        setError(null);
      })
      .catch(fail);
  };

  useEffect(refresh, []);

  // The two filters the task asks for, applied together. Derived rather than
  // stored, so a save or a delete cannot leave the list and the filters
  // disagreeing about what is in the library.
  const shown = useMemo(
    () =>
      saved.filter(
        (item) =>
          (artist === '' || item.artistId === artist) && (part === '' || item.part === part),
      ),
    [saved, artist, part],
  );

  const artists = useMemo(
    () => [...new Set(saved.map((item) => item.artistId))].sort(),
    [saved],
  );
  const parts = useMemo(() => [...new Set(saved.map((item) => item.part))].sort(), [saved]);

  const save = () => {
    const trimmed = name.trim();
    // ⚠ The clip on screen, not "the current pattern" — there are five slots and
    // the producer is saving the one they are looking at.
    const clip = patterns[activeTab as keyof typeof patterns];
    if (!trimmed || clip === undefined) return;
    invoke('pattern_save', {
      name: trimmed,
      // ⛔ **The clock is the page's.** Nothing in the engine or the plugin's
      // stores may depend on the time — the same rule the variation history
      // follows — so the timestamp is taken here and carried.
      savedAt: Date.now(),
      pattern: clip,
    })
      .then(() => {
        setName('');
        refresh();
      })
      .catch(fail);
  };

  const load = (item: PatternSummary) => {
    invoke<Pattern>('pattern_load', { id: item.id })
      .then((clip) => {
        // ⛔ **Through `openClip`, which is the same door an arrangement's clip
        // comes through.** It marks the slot edited — a loaded pattern is not
        // what this session's seed produces — and records one undo step, which
        // is what the task asks for.
        openClip(clip, clip.part);
        setError(null);
      })
      .catch(fail);
  };

  const remove = (id: string) => {
    invoke('pattern_delete', { id }).then(refresh).catch(fail);
  };

  /**
   * When it was saved, in the viewer's own locale.
   *
   * ⛔ `Intl`, never a hand-rolled format: this app ships 18 catalogs that pick
   * their own field order, their own month names and 24-hour time where that is
   * the convention — and two of them are right-to-left.
   */
  // ⛔ One formatter, not one per row — constructing an `Intl.DateTimeFormat`
  // is real work and the library can be long.
  const dates = useMemo(
    () => new Intl.DateTimeFormat(i18n.language, { dateStyle: 'medium' }),
    [i18n.language],
  );
  const when = (at: number) => (at > 0 ? dates.format(at) : '');

  return (
    <div className="patterns">
      {/* ⛔⛔ **Not native `<select>`s** (TASK-057) — a `<select>` popup inside
          WebView2 is drawn by the OS, against the window rather than the field.
          ⚠ The artist list grows with the library, so this is the pair most
          likely to reach the size Mike screenshotted. */}
      <div className="patterns__filters">
        <div className="patterns__filter">
          <Combo
            label={t('patterns.filterArtist')}
            options={[
              { id: '', name: t('patterns.allArtists') },
              ...artists.map((id) => ({ id, name: id })),
            ]}
            value={artist}
            onChange={setArtist}
          />
        </div>
        <div className="patterns__filter">
          <Combo
            label={t('patterns.filterPart')}
            options={[
              { id: '', name: t('patterns.allParts') },
              ...parts.map((id) => ({ id, name: t(`tabs.${id}`, id) })),
            ]}
            value={part}
            onChange={setPart}
          />
        </div>
      </div>

      <ul className="patterns__list">
        {shown.map((item) => (
          <li key={item.id} className="patterns__item">
            <button
              type="button"
              className="patterns__load"
              onClick={() => load(item)}
              title={t('patterns.load', { name: item.name })}
            >
              <span className="patterns__name">{item.name}</span>
              <MiniGrid density={item.density} />
              <span className="patterns__meta">
                {t('patterns.meta', {
                  artist: item.artistId,
                  bars: item.bars,
                  bpm: Math.round(item.bpm),
                })}
                {/* ⚠ Formatted once and reused — it was computed twice per row. */}
                {saved.length > 0 && when(item.savedAt) !== '' && ` · ${when(item.savedAt)}`}
              </span>
            </button>
            <button
              type="button"
              className="patterns__delete"
              aria-label={t('patterns.delete', { name: item.name })}
              onClick={() => remove(item.id)}
            >
              <Trash2 size={13} aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>

      {shown.length === 0 && !error && <p className="patterns__empty">{t('patterns.none')}</p>}

      <div className="patterns__save">
        <input
          type="text"
          className="patterns__input"
          value={name}
          maxLength={64}
          placeholder={t('patterns.name')}
          aria-label={t('patterns.name')}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') save();
          }}
        />
        <button
          type="button"
          // ⛔⛔ **`btn-ghost`, because `btn-secondary` was never defined.** Mike,
          // 2026-08-19: *"the save button over here needs to be an actual
          // button, not just a label."* It always **was** a `<button>` — it just
          // had no style: `btn-secondary` appeared exactly once in `src/` and
          // matched nothing in any stylesheet, so it rendered as bare text
          // beside the name box. ⚠ This is the same defect
          // `.btn-generate--secondary` records one file over — "applied since
          // TASK-120 and defined nowhere until 2026-08-15" — which is why the
          // fix comes with `classes.defined.test.ts` rather than alone.
          className="btn-ghost patterns__savebtn"
          // ⛔ Disabled with nothing on screen to save, rather than saving an
          // empty library row the producer would click and hear nothing from.
          disabled={
            name.trim() === '' || patterns[activeTab as keyof typeof patterns] === undefined
          }
          onClick={save}
        >
          {t('patterns.save')}
        </button>
      </div>

      {error !== null && <p className="patterns__error">{error}</p>}
    </div>
  );
}
