import { useEffect } from 'react';
import { X } from 'lucide-react';

import { AboutPane } from './Settings';
import './Settings.css';

/**
 * The standalone About overlay, reached from the ⓘ button in the title bar.
 *
 * Shares its body with Settings → About rather than restating it: the artist
 * disclaimer and the credits are legal text, and two copies would eventually
 * disagree about what the product claims.
 */
export function AboutModal({ onClose }: { onClose: () => void }) {
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
          <h2 id="about-title">About</h2>
          <button type="button" className="btn-ghost" aria-label="Close" onClick={onClose}>
            <X size={14} aria-hidden="true" />
          </button>
        </div>
        <AboutPane />
      </div>
    </div>
  );
}
