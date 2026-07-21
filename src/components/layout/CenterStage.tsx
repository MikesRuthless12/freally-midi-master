import { AudioWaveform, Drum, ListMusic, Music2, Piano, Waves } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { GENERATOR_TABS, useUi, type GeneratorTab } from '../../state/ui';

const TABS: Record<GeneratorTab, { label: string; Icon: LucideIcon }> = {
  drums: { label: 'Drums', Icon: Drum },
  melody: { label: 'Melody', Icon: Music2 },
  counter: { label: 'Counter', Icon: AudioWaveform },
  bass: { label: 'Bass', Icon: Waves },
  chords: { label: 'Chords', Icon: Piano },
  song: { label: 'Song', Icon: ListMusic },
};

function GeneratorTabs() {
  const activeTab = useUi((s) => s.activeTab);
  const setActiveTab = useUi((s) => s.setActiveTab);

  return (
    <div className="tabs" role="tablist" aria-label="Generator">
      {GENERATOR_TABS.map((tab) => {
        const { label, Icon } = TABS[tab];
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
            {label}
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
          <h2>Search an artist. Cook.</h2>
          <p>Pick someone from the roster, then hit Generate.</p>
        </div>

        <div className="stage__controls">
          <span className="chip chip--mono">
            seed <strong>—</strong>
          </span>
          <span className="chip chip--mono">
            <strong>4</strong> bars
          </span>
          <button type="button" className="btn-generate" disabled>
            Generate
          </button>
        </div>
      </div>
    </section>
  );
}
