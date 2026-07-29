import { X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
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
  const pattern = useSession((s) => s.pattern);
  const pins = useSession((s) => s.pins);
  const setPin = useSession((s) => s.setPin);
  const hostTempo = useSession((s) => s.hostTempo);

  if (!selectedId) {
    return <p className="session__empty">{t('session.pickArtist')}</p>;
  }

  /** What the artist chose last time, for the "leave it to them" option. */
  const chose = (value: string | null) =>
    value === null ? t('session.artistChoice') : t('session.artistPicks', { value });

  // Auto-sync, or set your own. The chip is empty while the tempo is being
  // followed and the placeholder shows what it is being followed *to*; typing
  // a number pins it and the host stops deciding. Clearing it hands the tempo
  // back to the project.
  const synced = pins.bpm === null && hostTempo !== null;
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
          title={synced ? t('session.hostSynced') : undefined}
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

      <label className="chip chip--mono session__chip">
        <span className="session__label">{t('readouts.key')}</span>
        <select
          className="session__select"
          value={pins.keyRoot ?? ''}
          onChange={(e) =>
            setPin('keyRoot', e.target.value === '' ? null : Number(e.target.value))
          }
        >
          <option value="">{chose(pattern ? keyName(pattern.keyRoot) : null)}</option>
          {KEY_NAMES.map((name, pitchClass) => (
            <option key={name} value={pitchClass}>
              {name}
            </option>
          ))}
        </select>
      </label>

      <label className="chip chip--mono session__chip">
        <span className="session__label">{t('readouts.scale')}</span>
        <select
          className="session__select"
          value={pins.scale ?? ''}
          onChange={(e) =>
            setPin('scale', e.target.value === '' ? null : (e.target.value as Scale))
          }
        >
          <option value="">{chose(pattern ? t(`scales.${pattern.scale}`) : null)}</option>
          {SCALES.map((scale) => (
            <option key={scale} value={scale}>
              {t(`scales.${scale}`)}
            </option>
          ))}
        </select>
      </label>

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
