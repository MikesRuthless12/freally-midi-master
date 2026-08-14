import { Link2, Unlink, Volume2, VolumeX, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { Combo } from '../Combo/Combo';
import { useSession, useActivePattern } from '../../state/session';
import { useSong } from '../../state/song';
import type { Scale } from '../../lib/ipc-types';
import {
  BPM_MAX,
  BPM_MIN,
  KEY_NAMES,
  SCALES,
  SWING_MAX,
  SWING_MIN,
  decimalOnly,
  digitsOnly,
  keyName,
  prettyKey,
} from './values';
import './SessionChips.css';

/**
 * The session, shown and editable (FR-002).
 *
 * Every chip is empty until it is pinned, and empty means *the artist decides*
 * — the same contract the seed box has, and the same one `SessionOverrides`
 * has in the engine. The placeholder shows what the artist asks for, so the
 * difference between "140 because I said so" and "140 because trap says so" is
 * visible rather than inferred.
 *
 * Key and scale have no placeholder before a generation, because a seed picks
 * them: the artist offers a list, and which one it lands on is not knowable
 * until Generate is pressed. Once it has been, the chip says which it chose.
 */
export function SessionChips() {
  const { t } = useTranslation();
  const selectedId = useSession((s) => s.selectedId);
  const defaults = useSession((s) => s.defaults);
  const active = useActivePattern();
  const patterns = useSession((s) => s.patterns);
  const song = useSong((s) => s.song);
  // ⛔ **The readout must not blank when the *tab* has nothing.** It used to be
  // the active tab's clip alone, so clicking Melody before generating it — or
  // opening Song, which is not a part at all — emptied the key, scale and mood
  // chips even though the session plainly had all three. The five parts share a
  // seed, so any loaded one reports the session's key; the arrangement reports
  // it when no part is loaded. Falling back is truthful rather than a guess.
  const pattern =
    active ??
    Object.values(patterns)[0] ??
    (song ? { keyRoot: song.keyRoot, scale: song.scale, mood: null } : null);
  const pins = useSession((s) => s.pins);
  const setPin = useSession((s) => s.setPin);
  const hostTempo = useSession((s) => s.hostTempo);
  const autoSync = useSession((s) => s.autoSync);
  const setAutoSync = useSession((s) => s.setAutoSync);
  const mood = useSession((s) => s.mood);
  const setMood = useSession((s) => s.setMood);
  const base = useSession((s) => s.base);
  const setBase = useSession((s) => s.setBase);
  const roster = useSession((s) => s.roster);
  const audioEnabled = useSession((s) => s.audioEnabled);
  const setAudioEnabled = useSession((s) => s.setAudioEnabled);

  if (!selectedId) {
    return <p className="session__empty">{t('session.pickArtist')}</p>;
  }

  // Absent rather than empty for a style with no `modes` block, which is most
  // of them — the field is skipped on the wire when there is nothing to send.
  const moods = defaults?.moods ?? [];

  /**
   * The genres this artist is listed under, for the base chip (TASK-158C).
   *
   * ⛔ **`relatedGenres`, because that is exactly what `cross-filter.ts`
   * filters the rail on.** Offering a different list here would be a chip that
   * disagreed with the roster about which genres this artist works in, which is
   * the readout-that-lies failure the whole task is closing — arriving through
   * the fix.
   *
   * ⚠ **Named from the roster rather than shown as an id.** `boom-bap` is a key,
   * not a label; the genre's own entry is what carries the name a producer
   * reads. An id that resolves to nothing is dropped — the plugin already drops
   * dangling `relatedGenres` from the roster, so this only ever loses one the
   * rail is not offering either.
   *
   * ⛔ **Empty for a genre, and that is a decision rather than an accident of
   * the data.** 36 of the 56 shipped genres carry `relatedGenres` too, so
   * without this the chip would appear over Trap offering to generate "Trap, in
   * Drill". `resolve_over` would answer something for that, but the feature is
   * *"an artist generating in every genre they work in"* — a genre generating
   * in another genre is a control whose meaning nobody asked for and nobody
   * could predict.
   */
  const own = roster.find((entry) => entry.id === selectedId);
  const relatedGenres = (own?.type === 'genre' ? [] : (own?.relatedGenres ?? [])).flatMap(
    (id) => {
      const genre = roster.find((entry) => entry.id === id);
      return genre ? [{ id, name: genre.name }] : [];
    },
  );

  /** What the artist chose last time, for the "leave it to them" option. */
  const chose = (value: string | null) =>
    value === null ? t('session.artistChoice') : t('session.artistPicks', { value });

  // Auto-sync, or set your own. The chip is empty while the tempo is being
  // followed and the placeholder shows what it is being followed *to*; typing
  // a number pins it and the host stops deciding. Clearing it hands the tempo
  // back to the project.
  // ⛔ Three states, not two (TASK-P15). The pin distinguishes "mine" from
  // "not mine"; `autoSync` distinguishes the two kinds of "not mine" — the
  // DAW's tempo and the artist's own. Before the toggle existed the artist's
  // was unreachable in a host, and a chip that showed the host's number while
  // generating at the artist's would be the readout lying either way.
  const synced = pins.bpm === null && hostTempo !== null && autoSync;
  const tempoPlaceholder = synced
    ? String(Math.round(hostTempo))
    : defaults
      ? String(Math.round(defaults.bpm))
      : '—';

  return (
    <div className="readouts session">
      <label className="chip chip--mono session__chip" data-synced={synced || undefined}>
        <span className="session__label">{t('readouts.bpm')}</span>
        <input
          className="session__number"
          // Text, not `number`. A number input accepts `e`, `E`, `+`, `-` and
          // `.` for scientific notation — "1e5" is a legal value that arrives
          // as 100000 — and when it holds something the browser calls invalid
          // it reports an empty value, which reads here as "unpinned". Digits
          // are filtered on the way in instead, so nothing else can be typed.
          type="text"
          inputMode="numeric"
          // No `maxLength`: the browser applies it to the raw keystrokes,
          // *before* the filter below runs, so typing "12e5" fills up on "12e"
          // and the 5 is dropped entirely. Filter first, then limit — three
          // digits, because the ceiling is 999.
          value={pins.bpm ?? ''}
          placeholder={tempoPlaceholder}
          title={
            synced
              ? t('session.hostSynced')
              : hostTempo !== null && !autoSync
                ? t('session.autoSyncOff')
                : undefined
          }
          onChange={(e) => {
            const digits = digitsOnly(e.target.value).slice(0, 3);
            setPin('bpm', digits === '' ? null : Number(digits));
          }}
          // Clamped when the field is left rather than on each keystroke, or
          // typing "5" on the way to "50" would be corrected under the cursor.
          // Anything that generates takes focus away first, so the number shown
          // is always the number the engine will use.
          onBlur={(e) => {
            const digits = digitsOnly(e.target.value);
            if (digits === '') return;
            setPin('bpm', Math.min(BPM_MAX, Math.max(BPM_MIN, Number(digits))));
          }}
        />
        <Unpin field="bpm" pinned={pins.bpm !== null} />
      </label>

      {/* ⛔ Only shown inside a host, because there is nothing to sync to
          otherwise — the standalone has no project. A toggle that was present
          and inert would be a control that can only do nothing, which is the
          rule the factory-preset delete button follows too. */}
      {hostTempo !== null && (
        <button
          type="button"
          className="chip session__sync"
          role="switch"
          aria-checked={autoSync}
          title={autoSync ? t('session.hostSynced') : t('session.autoSyncOff')}
          onClick={() => setAutoSync(!autoSync)}
        >
          {autoSync ? (
            <Link2 size={12} aria-hidden="true" />
          ) : (
            <Unlink size={12} aria-hidden="true" />
          )}
          <span className="session__label">{t('session.autoSync')}</span>
        </button>
      )}

      {/* ⛔ Always offered, unlike the DAW-sync switch above it. That one is
          about a host that may not exist; this is about the plugin's own sound,
          which it makes in a DAW and in the standalone alike. MIDI-only is a
          first-class mode — it is what the plugin did before it had a sampler,
          and a producer routing into their own drums needs it in one click. */}
      <button
        type="button"
        className="chip session__sync"
        role="switch"
        aria-checked={audioEnabled}
        title={audioEnabled ? t('session.audioOn') : t('session.audioOff')}
        onClick={() => setAudioEnabled(!audioEnabled)}
      >
        {audioEnabled ? (
          <Volume2 size={12} aria-hidden="true" />
        ) : (
          <VolumeX size={12} aria-hidden="true" />
        )}
        <span className="session__label">{t('session.audio')}</span>
      </button>

      {/* ⛔⛔ **Not native `<select>`s** (TASK-057). A `<select>` popup inside
          WebView2 is drawn by the OS, against the *window* rather than the
          field and at OS scale — Mike screenshotted it. The scale chip below
          offers **41** scales, which is the list length that produced the
          screenshot; it was the strongest case in the app for this change and
          it was missing from the "six left" count entirely.
          ⚠ `<div>` rather than `<label>`: a `<label>` wrapping a combobox
          refocuses the input on every click inside it, including on the arrow
          whose whole job is to toggle the list. `Combo`'s `label` carries the
          accessible name the `<label>` used to give, so the name is unchanged. */}
      <div className="chip chip--mono session__chip">
        <span className="session__label">{t('readouts.key')}</span>
        <Combo
          label={t('readouts.key')}
          // ⛔ First, and it is the default: absence means "the artist chooses".
          // It is a real option rather than an empty field, because `Combo`'s
          // contract is that you cannot end up with nothing selected — and here
          // "nothing pinned" is itself a choice worth being able to make again.
          options={[
            { id: '', name: chose(pattern ? keyName(pattern.keyRoot) : null) },
            ...KEY_NAMES.map((name, pitchClass) => ({ id: String(pitchClass), name })),
          ]}
          value={pins.keyRoot === null ? '' : String(pins.keyRoot)}
          onChange={(id) => setPin('keyRoot', id === '' ? null : Number(id))}
        />
      </div>

      <div className="chip chip--mono session__chip">
        <span className="session__label">{t('readouts.scale')}</span>
        <Combo
          label={t('readouts.scale')}
          options={[
            { id: '', name: chose(pattern ? t(`scales.${pattern.scale}`) : null) },
            ...SCALES.map((scale) => ({ id: scale, name: t(`scales.${scale}`) })),
          ]}
          value={pins.scale ?? ''}
          onChange={(id) => setPin('scale', id === '' ? null : (id as Scale))}
        />
      </div>

      {/* ⛔⛔ **"Drake, but in R&B"** (TASK-158C). The roster lists an artist
          under every genre in their `relatedGenres` — 529 of 534 models name
          one they do not `extend` — and Generate has always answered the one
          they do. This chip is what makes the rail's claim true.

          ⛔ **A pin of its own rather than the genre combobox changing
          meaning.** That box and the roster box both write `selectedId`; making
          one of them mean something else when an artist is selected is a
          control whose behaviour depends on state you cannot see. "Any" here
          means the artist's own base, which is what "Any" means in every other
          chip in this row.

          ⚠ **Only where there is a choice**, on the same rule as the mood chip
          beside it: an artist who works in one genre has nothing to pick
          between, and a combobox with one option is a control that cannot do
          anything. */}
      {relatedGenres.length > 0 && (
        <div className="chip chip--mono session__chip">
          <span className="session__label">{t('readouts.base')}</span>
          <Combo
            label={t('readouts.base')}
            options={[
              { id: '', name: t('readouts.ownGenre') },
              ...relatedGenres.map((genre) => ({
                id: genre.id,
                name: genre.name,
              })),
            ]}
            value={base ?? ''}
            onChange={(id) => setBase(id === '' ? null : id)}
          />
        </div>
      )}

      {/* Only for a style that offers modes — eleven of the shipped genres
          author none, and a chip whose only option is "Any" is a control that
          cannot do anything. "Any" is a *pick from the seed* rather than "no
          mood", so a reroll can land on a different kind of record by the same
          artist; the chip then says which one it landed on, exactly as the
          key and scale chips do. */}
      {moods.length > 0 && (
        <div className="chip chip--mono session__chip">
          <span className="session__label">{t('readouts.mood')}</span>
          <Combo
            label={t('readouts.mood')}
            options={[
              { id: '', name: chose(pattern?.mood ?? null) },
              ...moods.map((name) => ({ id: name, name })),
            ]}
            value={mood ?? ''}
            onChange={(id) => setMood(id === '' ? null : id)}
          />
        </div>
      )}

      <label className="chip chip--mono session__chip">
        <span className="session__label">{t('readouts.swing')}</span>
        <input
          className="session__number"
          // Text for the same reason as the tempo, plus one of its own: swing
          // is fractional, and a number input's locale handling turns "0,54"
          // into an empty value in half of Europe.
          type="text"
          inputMode="decimal"
          value={pins.swing ?? ''}
          placeholder={defaults ? defaults.swing.amount.toFixed(2) : '—'}
          onChange={(e) => {
            // Filtered then limited, for the same reason as the tempo above.
            const cleaned = decimalOnly(e.target.value).slice(0, 4);
            setPin('swing', cleaned === '' ? null : Number(cleaned));
          }}
          onBlur={(e) => {
            const cleaned = decimalOnly(e.target.value);
            if (cleaned === '') return;
            setPin('swing', Math.min(SWING_MAX, Math.max(SWING_MIN, Number(cleaned))));
          }}
        />
        <Unpin field="swing" pinned={pins.swing !== null} />
      </label>
    </div>
  );
}

/**
 * Hand a field back to the artist.
 *
 * Only rendered for the two number fields: a select already has an option that
 * means "the artist's", and a second control for the same thing beside it
 * would be one more place for the two to disagree.
 */
function Unpin({ field, pinned }: { field: 'bpm' | 'swing'; pinned: boolean }) {
  const { t } = useTranslation();
  const setPin = useSession((s) => s.setPin);
  if (!pinned) return null;

  return (
    <button
      type="button"
      className="btn-ghost session__unpin"
      onClick={() => setPin(field, null)}
      aria-label={t('session.clearPin')}
      title={t('session.clearPin')}
    >
      <X size={12} aria-hidden="true" />
    </button>
  );
}

/**
 * Keep the pinned session, or adopt the new artist's (FR-002).
 *
 * It sits by the Generate button rather than beside the chips, because the
 * right rail collapses under 1440px and behind K — a prompt nobody can see
 * would leave the last artist's tempo quietly attached to this one. It does
 * not block: the artist has already changed, and browsing a roster must not
 * cost a dialog per click. Keeping is the default the PRD states, so ignoring
 * it entirely loses nothing.
 */
export function SessionSwitchPrompt() {
  const { t } = useTranslation();
  const pending = useSession((s) => s.pendingArtist);
  const pins = useSession((s) => s.pins);
  const defaults = useSession((s) => s.defaults);
  const keepPins = useSession((s) => s.keepPins);
  const adoptDefaults = useSession((s) => s.adoptDefaults);

  if (!pending) return null;

  // Only the pinned rows: an unpinned field is not in dispute, and listing it
  // would make the switch look bigger than it is.
  const rows: { label: string; mine: string; theirs: string }[] = [];
  if (pins.bpm !== null) {
    rows.push({
      label: t('readouts.bpm'),
      mine: String(pins.bpm),
      theirs: defaults ? String(Math.round(defaults.bpm)) : '—',
    });
  }
  if (pins.keyRoot !== null) {
    rows.push({
      label: t('readouts.key'),
      mine: keyName(pins.keyRoot) ?? '—',
      theirs: defaults?.keys.length ? defaults.keys.map(prettyKey).join(' / ') : '—',
    });
  }
  if (pins.scale !== null) {
    rows.push({
      label: t('readouts.scale'),
      mine: t(`scales.${pins.scale}`),
      theirs: defaults?.scales.length
        ? defaults.scales.map((scale) => t(`scales.${scale}`)).join(' / ')
        : '—',
    });
  }
  if (pins.swing !== null) {
    rows.push({
      label: t('readouts.swing'),
      mine: pins.swing.toFixed(2),
      theirs: defaults ? defaults.swing.amount.toFixed(2) : '—',
    });
  }

  return (
    <div className="switch-prompt" role="status">
      <p className="switch-prompt__body">{t('session.switchBody', { name: pending.name })}</p>
      <ul className="switch-prompt__rows">
        {rows.map((row) => (
          <li key={row.label}>
            <span className="switch-prompt__field">{row.label}</span>
            <strong>{row.mine}</strong>
            <span aria-hidden="true">→</span>
            <span>{row.theirs}</span>
          </li>
        ))}
      </ul>
      <div className="switch-prompt__actions">
        <button type="button" className="btn-ghost" onClick={keepPins}>
          {t('session.keep')}
        </button>
        <button type="button" className="btn-ghost" onClick={adoptDefaults}>
          {t('session.adopt', { name: pending.name })}
        </button>
      </div>
    </div>
  );
}
