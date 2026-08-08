import { useEffect } from 'react';
import { X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { SHORTCUT_GROUPS } from './catalog';
import '../Settings/Settings.css';
import './Shortcuts.css';

/**
 * The keyboard-shortcuts panel (TASK-131I).
 *
 * ⚠ **Shares `Settings.css`'s overlay rather than restating it**, for the same
 * reason `AboutModal` does: three dialogs with three slightly different scrims
 * is how one of them ends up unreachable behind the rail.
 *
 * ⛔ **Escape closes it, and that is not merely conventional here.** This panel
 * is opened from a keyboard shortcut, so a producer who opens it has their
 * hands on the keys — leaving it dismissable only by mouse would be the one
 * dialog in the app you have to reach for the mouse to leave.
 */
export function ShortcutsModal({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div className="settings" role="dialog" aria-modal="true" aria-labelledby="shortcuts-title">
      <div className="about__panel shortcuts__panel">
        <div className="about__head">
          <h2 id="shortcuts-title">{t('shortcuts.title')}</h2>
          <button
            type="button"
            className="btn-ghost"
            aria-label={t('common.close')}
            onClick={onClose}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>

        <div className="shortcuts__body">
          {SHORTCUT_GROUPS.map((group) => (
            <section key={group.id} className="shortcuts__group">
              <h3 className="shortcuts__groupName">{t(`shortcuts.groups.${group.id}`)}</h3>
              <dl className="shortcuts__list">
                {group.items.map((item) => (
                  <div key={item.id} className="shortcuts__row">
                    <dt className="shortcuts__what">{t(`shortcuts.keys.${item.id}`)}</dt>
                    {/* The keys second in the DOM but first visually, so a
                        screen reader reads "Select all — Ctrl + A" rather than
                        a stream of modifiers with no verbs attached. */}
                    <dd className="shortcuts__keys">
                      <kbd>{item.keys}</kbd>
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
