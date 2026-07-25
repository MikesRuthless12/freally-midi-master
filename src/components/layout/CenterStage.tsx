import { useMemo } from 'react';
import { AudioWaveform, Drum, ListMusic, Music2, Piano, Waves } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { GENERATOR_TABS, useUi, type GeneratorTab } from '../../state/ui';
import { BAR_CHOICES, useSession } from '../../state/session';
import { DrumGrid } from '../DrumGrid/DrumGrid';
import { columnDensity } from '../DrumGrid/cells';
import { GenFx } from '../GenFx/GenFx';
import { SeedChip } from '../SeedChip/SeedChip';
import { useTranslation } from 'react-i18next';

/** Density buckets handed to the ripple. Matches the columns it draws. */
const FX_COLUMNS = 64;

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
 * Centre stage: the tab strip over the grid.
 *
 * Only the Drums tab generates. The other five are Phase 2, and they say so
 * rather than showing an empty grid that reads as a failed generation.
 */
export function CenterStage() {
  const { t } = useTranslation();
  const activeTab = useUi((s) => s.activeTab);

  const selectedId = useSession((s) => s.selectedId);
  const roster = useSession((s) => s.roster);
  const pattern = useSession((s) => s.pattern);
  const bars = useSession((s) => s.bars);
  const setBars = useSession((s) => s.setBars);
  const generating = useSession((s) => s.generating);
  const error = useSession((s) => s.error);
  const generate = useSession((s) => s.generate);
  const playhead = useSession((s) => s.playhead);

  const selected = roster.find((entry) => entry.id === selectedId) ?? null;
  const isDrums = activeTab === 'drums';

  // What the ripple ignites. Recomputed only when the pattern changes, not on
  // every frame — the animation reads this, and it must not cost a render.
  const density = useMemo(
    () => (pattern ? columnDensity(pattern, FX_COLUMNS) : undefined),
    [pattern],
  );

  return (
    <section className="stage">
      <GeneratorTabs />

      <div
        className="stage__body"
        role="tabpanel"
        id="generator-panel"
        aria-labelledby={`tab-${activeTab}`}
      >
        {/* The ripple wraps whatever the stage is showing, so it sweeps the
            grid the notes are landing in rather than a layer beside it. */}
        <GenFx active={generating} density={density}>
          {!isDrums ? (
            <div className="stage__empty">
              <h2>{t(`tabs.${activeTab}`)}</h2>
              <p>{t('stage.laterPhase')}</p>
            </div>
          ) : pattern ? (
            <DrumGrid pattern={pattern} playhead={playhead} />
          ) : (
            <div className="stage__empty">
              <h2>{t('stage.emptyTitle')}</h2>
              <p>
                {selected
                  ? t('stage.readyBody', { name: selected.name })
                  : t('stage.emptyBody')}
              </p>
            </div>
          )}
        </GenFx>

        {/* The error sits beside the control that caused it rather than in a
            toast that has to be chased across the screen. */}
        {error && (
          <p className="stage__error" role="alert">
            {error}
          </p>
        )}

        <div className="stage__controls">
          <SeedChip />

          <span className="chip chip--mono" role="group" aria-label={t('stage.barsLabel')}>
            {BAR_CHOICES.map((choice) => (
              <button
                key={choice}
                type="button"
                className="chip__option"
                aria-pressed={bars === choice}
                onClick={() => setBars(choice)}
              >
                {choice}
              </button>
            ))}
            {t('stage.bars')}
          </span>

          <button
            type="button"
            className="btn-generate"
            onClick={() => void generate()}
            disabled={!isDrums || !selectedId || generating}
          >
            {generating ? t('stage.generating') : t('stage.generate')}
          </button>
        </div>
      </div>
    </section>
  );
}
