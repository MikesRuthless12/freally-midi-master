import { useEffect, useRef, useState } from 'react';
import { Check, Eye } from 'lucide-react';
import { SECTIONS, useUi } from '../../state/ui';
import { useTranslation } from 'react-i18next';

/**
 * Discoverability for the collapsible panels. The chevrons on each section are
 * the fast path; this is the one place that lists every panel, so a section
 * collapsed and forgotten can always be found again.
 */
export function ViewMenu() {
  const { t } = useTranslation();
  const sections = useUi((s) => s.sections);
  const toggleSection = useUi((s) => s.toggleSection);
  const setAllSections = useUi((s) => s.setAllSections);
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const toggleRightRail = useUi((s) => s.toggleRightRail);

  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const allOpen = SECTIONS.every((id) => sections[id]) && rightRailOpen;

  return (
    <div className="viewmenu" ref={wrap}>
      <button
        type="button"
        className="btn-ghost"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <Eye size={14} aria-hidden="true" />
        {t('view.button')}
      </button>

      {open && (
        <div className="viewmenu__pop" role="menu" aria-label={t('view.panels')}>
          <button
            type="button"
            role="menuitemcheckbox"
            aria-checked={rightRailOpen}
            className="viewmenu__item"
            onClick={toggleRightRail}
          >
            <span className="viewmenu__check">{rightRailOpen && <Check size={12} />}</span>
            {t('view.rightRail')}
            <kbd className="viewmenu__kbd">K</kbd>
          </button>

          <div className="viewmenu__sep" role="separator" />

          {SECTIONS.map((id) => (
            <button
              key={id}
              type="button"
              role="menuitemcheckbox"
              aria-checked={sections[id]}
              className="viewmenu__item"
              onClick={() => toggleSection(id)}
            >
              <span className="viewmenu__check">{sections[id] && <Check size={12} />}</span>
              {t(`sections.${id}`)}
            </button>
          ))}

          <div className="viewmenu__sep" role="separator" />

          <button
            type="button"
            role="menuitem"
            className="viewmenu__item"
            onClick={() => {
              setAllSections(!allOpen);
              if (rightRailOpen !== !allOpen) toggleRightRail();
            }}
          >
            <span className="viewmenu__check" />
            {allOpen ? t('view.hideAll') : t('view.showAll')}
          </button>
        </div>
      )}
    </div>
  );
}
