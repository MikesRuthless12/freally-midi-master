import { useTranslation } from 'react-i18next';

import { SCALES } from '../SessionChips/values';
import { useEditing } from '../../state/editing';
import { useSession } from '../../state/session';
import type { Pattern, Scale } from '../../lib/ipc-types';
import { SNAP_VALUES, type Snap } from './notes';

/**
 * The roll's own header: snap, the two folds, and the scale (TASK-041B).
 *
 * ⛔ **The scale picker writes through to the session, not to a second copy.**
 * `SessionChips` already owns the key and scale a generation runs in, and the
 * roll drawing its rows from a *different* opinion about the key is the
 * readout-that-lies failure this codebase keeps closing — the tinted rows would
 * say one thing and the next Generate would produce another. So this sets
 * `pins.scale`, and the chip and the roll are two views of one value.
 *
 * The meter picker (TASK-041E) follows the same rule, with one addition: it
 * also writes the *pattern*, because the pattern is what exports.
 */

/**
 * The meters the picker offers.
 *
 * A list rather than two number boxes: a producer picking a meter is choosing
 * between the ones music is written in, and a free numerator invites 13/7 and
 * the `0` that `engine::context` has to defend against. Whatever the clip
 * actually arrived in is added to the list, so a project in a meter this forgot
 * still shows its own rather than silently reading as 4/4.
 */
const METERS = ['4/4', '3/4', '2/4', '6/8', '9/8', '12/8', '5/4', '7/8'];

export function RollBar({
  scale,
  pattern,
  children,
}: {
  scale: Scale;
  pattern: Pattern;
  children?: React.ReactNode;
}) {
  const { t } = useTranslation();

  const snap = useEditing((s) => s.snap);
  const setSnap = useEditing((s) => s.setSnap);
  const foldToScale = useEditing((s) => s.foldToScale);
  const setFoldToScale = useEditing((s) => s.setFoldToScale);
  const foldToNotes = useEditing((s) => s.foldToNotes);
  const setFoldToNotes = useEditing((s) => s.setFoldToNotes);
  const setPin = useSession((s) => s.setPin);
  const editPattern = useSession((s) => s.editPattern);

  const meter = `${pattern.timeSigNum}/${pattern.timeSigDen}`;
  const meters = METERS.includes(meter) ? METERS : [meter, ...METERS];
  const setMeter = (value: string) => {
    const [num, den] = value.split('/').map(Number);
    if (!(num > 0) || !(den > 0)) return;
    editPattern({ ...pattern, timeSigNum: num, timeSigDen: den });
    setPin('timeSigNum', num);
    setPin('timeSigDen', den);
  };

  return (
    <div className="rollbar">
      <label className="rollbar__field">
        <span className="rollbar__label">{t('roll.snapLabel')}</span>
        <select
          className="rollbar__select"
          value={snap}
          onChange={(event) => setSnap(event.target.value as Snap)}
        >
          {SNAP_VALUES.map((value) => (
            // ⛔ Only `off` is translated. `1/16` and `1/8T` are notation, not
            // words — a locale that rendered them differently would be showing
            // a producer a grid they cannot name.
            <option key={value} value={value}>
              {value === 'off' ? t('roll.snapOff') : value}
            </option>
          ))}
        </select>
      </label>

      <label className="rollbar__field">
        <span className="rollbar__label">{t('roll.scaleLabel')}</span>
        <select
          className="rollbar__select"
          value={scale}
          onChange={(event) => setPin('scale', event.target.value as Scale)}
        >
          {SCALES.map((value) => (
            <option key={value} value={value}>
              {t(`scales.${value}`)}
            </option>
          ))}
        </select>
      </label>

      {/*
        The clip's meter (TASK-041E).

        ⛔ **It writes the pattern *and* the pin.** The pattern is what exports —
        `engine::midi` reads `timeSig*` for the file's own meta event — so
        changing only the pin would leave a clip that says 6/8 on screen and
        opens as 4/4 in a DAW. The pin is what the next Generate inherits, and
        `host.rs::session_for` lets it beat the host's own project meter.
      */}
      <label className="rollbar__field">
        <span className="rollbar__label">{t('roll.meterLabel')}</span>
        <select
          className="rollbar__select"
          value={meter}
          onChange={(event) => setMeter(event.target.value)}
        >
          {meters.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      </label>

      <button
        type="button"
        className="rollbar__toggle"
        aria-pressed={foldToScale}
        onClick={() => setFoldToScale(!foldToScale)}
        title={t('roll.foldScaleHint')}
      >
        {t('roll.foldScale')}
      </button>

      <button
        type="button"
        className="rollbar__toggle"
        aria-pressed={foldToNotes}
        onClick={() => setFoldToNotes(!foldToNotes)}
        title={t('roll.foldNotesHint')}
      >
        {t('roll.foldNotes')}
      </button>

      {/* The transform menu, passed in rather than built here: it needs the
          pattern and the key, and the bar is a bar. */}
      {children}
    </div>
  );
}
