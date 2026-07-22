import { useEffect } from 'react';
import { X } from 'lucide-react';

import { AboutPane } from './Settings';
import './Settings.css';
import { useTranslation } from 'react-i18next';

/**
 * The standalone About overlay, reached from the ⓘ button in the title bar.
 *
 * Shares its body with Settings → About rather than restating it: the artist
 * disclaimer and the credits are legal text, and two copies would eventually
 * disagree about what the product claims.
 */
export function AboutModal({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div className="settings" role="dialog" aria-modal="true" aria-labelledby="about-title">
      <div className="about__panel">
        <div className="about__head">
          <h2 id="about-title">{t('about.title')}</h2>
          <button
            type="button"
            className="btn-ghost"
            aria-label={t('common.close')}
            onClick={onClose}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
        <AboutPane />
      </div>
    </div>
  );
}
