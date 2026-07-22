import { Section } from './Section';
import { useTranslation } from 'react-i18next';

/**
 * Right rail: kit over session readouts. The rail as a whole collapses below
 * 1440px and toggles with K; each panel inside also collapses on its own.
 */
export function RightRail() {
  const { t } = useTranslation();

  return (
    <aside className="rail rail--right">
      <Section id="kit" grow>
        <div className="pads">
          {Array.from({ length: 8 }, (_, i) => (
            <button key={i} type="button" className="pad" disabled>
              {i + 1}
            </button>
          ))}
        </div>
        <div className="kit-drop">{t('rails.noKit')}</div>
      </Section>

      <Section id="session">
        <div className="readouts">
          <span className="chip chip--mono">
            {t('readouts.bpm')} <strong>—</strong>
          </span>
          <span className="chip chip--mono">
            {t('readouts.key')} <strong>—</strong>
          </span>
          <span className="chip chip--mono">
            {t('readouts.swing')} <strong>—</strong>
          </span>
        </div>
      </Section>
    </aside>
  );
}
