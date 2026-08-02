import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useEditing } from '../../state/editing';
import { useSession } from '../../state/session';
import type { Lane, Pattern } from '../../lib/ipc-types';
import { stepTicks } from './notes';
import {
  humanize,
  invert,
  legato,
  quantize,
  reselect,
  reverse,
  stretch,
  transposeToScale,
} from './transforms';

/**
 * The transform menu (TASK-041D).
 *
 * ⛔ **Every item is one `editPattern` call, and therefore one undo step.** A
 * transform that wrote twice — say, edit then re-select — would cost two presses
 * of Ctrl+Z to take back one thing the producer did once, which is the failure
 * `history.ts` is built to avoid.
 *
 * ⛔ **The selection is re-derived after every transform.** A `NoteId` is a start
 * tick and a pitch, and these move both — so without it the second item in a
 * chain would act on notes that no longer exist and the menu would appear to
 * stop working after one use.
 */

/** How far `Humanize` may move a note: an eighth of the grid, and ±10 velocity. */
const HUMANIZE_TICKS = 8;
const HUMANIZE_VELOCITY = 10;

export function TransformMenu({
  pattern,
  lane,
  classes,
}: {
  pattern: Pattern;
  lane: Lane;
  /** The pitch classes in the key, or `null` before the engine has answered. */
  classes: ReadonlySet<number> | null;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);

  const editPattern = useSession((s) => s.editPattern);
  const selection = useEditing((s) => s.selection);
  const snap = useEditing((s) => s.snap);
  const strength = useEditing((s) => s.quantizeStrength);
  const setStrength = useEditing((s) => s.setQuantizeStrength);
  const empty = selection.size === 0;

  // Close on Escape or on a click outside, the two ways anyone dismisses a menu.
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    const onDown = (event: MouseEvent) => {
      if (!boxRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', onDown);
    return () => {
      window.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onDown);
    };
  }, [open]);

  const run = useCallback(
    (transform: (current: Pattern) => Pattern) => {
      const next = transform(pattern);
      if (next === pattern) return;
      editPattern(next);
      useEditing.getState().select(reselect(pattern, next, lane, selection));
      setOpen(false);
    },
    [pattern, lane, selection, editPattern],
  );

  const items: { key: string; label: string; run: () => void; enabled: boolean }[] = [
    {
      key: 'invertUp',
      label: t('roll.invertUp'),
      run: () => run((p) => invert(p, lane, selection, 'up')),
      enabled: !empty,
    },
    {
      key: 'invertDown',
      label: t('roll.invertDown'),
      run: () => run((p) => invert(p, lane, selection, 'down')),
      enabled: !empty,
    },
    {
      key: 'reverse',
      label: t('roll.reverse'),
      run: () => run((p) => reverse(p, lane, selection)),
      enabled: !empty,
    },
    {
      key: 'stretch',
      label: t('roll.stretch'),
      run: () => run((p) => stretch(p, lane, selection, 2)),
      enabled: !empty,
    },
    {
      key: 'compress',
      label: t('roll.compress'),
      run: () => run((p) => stretch(p, lane, selection, 0.5)),
      enabled: !empty,
    },
    {
      key: 'legato',
      label: t('roll.legato'),
      run: () => run((p) => legato(p, lane, selection)),
      enabled: !empty,
    },
    {
      key: 'quantize',
      label: t('roll.quantize'),
      run: () => run((p) => quantize(p, lane, selection, stepTicks(snap, p.ppq), strength)),
      enabled: !empty && snap !== 'off',
    },
    {
      key: 'humanize',
      label: t('roll.humanize'),
      run: () =>
        run((p) =>
          humanize(p, lane, selection, {
            ticks: HUMANIZE_TICKS,
            velocity: HUMANIZE_VELOCITY,
          }),
        ),
      enabled: !empty,
    },
    {
      key: 'toScale',
      label: t('roll.toScale'),
      // ⛔ Off until the engine has answered with the key's pitch classes. A
      // fold against an empty set would be a no-op that looked like a bug.
      run: () =>
        run((p) => (classes === null ? p : transposeToScale(p, lane, selection, classes))),
      enabled: !empty && classes !== null,
    },
  ];

  return (
    <div className="rollmenu" ref={boxRef}>
      <button
        type="button"
        className="rollbar__toggle"
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={() => setOpen((was) => !was)}
      >
        {t('roll.transform')}
      </button>

      {open && (
        <div className="rollmenu__list" role="menu" aria-label={t('roll.transform')}>
          {items.map((item) => (
            <button
              key={item.key}
              type="button"
              role="menuitem"
              className="rollmenu__item"
              disabled={!item.enabled}
              title={item.enabled ? undefined : t('roll.selectFirst')}
              onClick={item.run}
            >
              {item.label}
            </button>
          ))}

          {/* The strength the roadmap asks for, beside the item it belongs to
              rather than in a dialog of its own. */}
          <label className="rollmenu__field">
            <span>{t('roll.quantizeStrength')}</span>
            <input
              type="range"
              min={0}
              max={100}
              step={5}
              value={Math.round(strength * 100)}
              onChange={(event) => setStrength(Number(event.target.value) / 100)}
            />
            <output>{Math.round(strength * 100)}%</output>
          </label>
        </div>
      )}
    </div>
  );
}
