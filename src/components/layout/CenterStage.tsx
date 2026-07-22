import { AudioWaveform, Drum, ListMusic, Music2, Piano, Waves } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { GENERATOR_TABS, useUi, type GeneratorTab } from '../../state/ui';
import { useTranslation } from 'react-i18next';

/** Icons only — every label comes from the catalog, keyed by tab id. */
const TAB_ICONS: Record<GeneratorTab, LucideIcon> = {
  drums: Drum,
  melody: Music2,
  counter: AudioWaveform,
  bass: Waves,
  chords: Piano,
  song: ListMusic,
};

function GeneratorTabs() {
  const { t } = useTranslation();
  const activeTab = useUi((s) => s.activeTab);
  const setActiveTab = useUi((s) => s.setActiveTab);

  return (
    <div className="tabs" role="tablist" aria-label={t('tabs.group')}>
      {GENERATOR_TABS.map((tab) => {
        const Icon = TAB_ICONS[tab];
        const selected = tab === activeTab;
        return (
          <button
            key={tab}
            type="button"
            role="tab"
            id={`tab-${tab}`}
            aria-selected={selected}
            aria-controls="generator-panel"
            tabIndex={selected ? 0 : -1}
            className="tab"
            onClick={() => setActiveTab(tab)}
          >
            <Icon size={16} aria-hidden="true" />
            {t(`tabs.${tab}`)}
          </button>
        );
      })}
    </div>
  );
}

/**
 * Centre stage: the tab strip over the grid. The grid itself is a placeholder
 * until the drum sequencer and piano roll land in Phase 1.
 */
export function CenterStage() {
  const { t } = useTranslation();
  const activeTab = useUi((s) => s.activeTab);

  return (
    <section className="stage">
      <GeneratorTabs />

      <div
        className="stage__body"
        role="tabpanel"
        id="generator-panel"
        aria-labelledby={`tab-${activeTab}`}
      >
        <div className="stage__empty">
          <h2>{t('stage.emptyTitle')}</h2>
          <p>{t('stage.emptyBody')}</p>
        </div>

        <div className="stage__controls">
          <span className="chip chip--mono">
            {t('stage.seed')} <strong>—</strong>
          </span>
          <span className="chip chip--mono">
            <strong>4</strong> {t('stage.bars')}
          </span>
          <button type="button" className="btn-generate" disabled>
            {t('stage.generate')}
          </button>
        </div>
      </div>
    </section>
  );
}
