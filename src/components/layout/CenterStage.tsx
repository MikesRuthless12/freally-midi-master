import { useMemo } from 'react';
import { AudioWaveform, Drum, ListMusic, Music2, Piano, Waves } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { GENERATOR_TABS, useUi, type GeneratorTab } from '../../state/ui';
import { BAR_CHOICES, useSession } from '../../state/session';
import { DrumGrid } from '../DrumGrid/DrumGrid';
import { PianoRoll } from '../PianoRoll/PianoRoll';
import { SongTimeline } from '../SongTimeline/SongTimeline';
import { useSong } from '../../state/song';
import { columnDensity } from '../DrumGrid/cells';
import type { Part } from '../../lib/ipc-types';
import { GenFx } from '../GenFx/GenFx';
import { SeedChip } from '../SeedChip/SeedChip';
import { SessionSwitchPrompt } from '../SessionChips/SessionChips';
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
 * Which part a tab generates, or `null` for a tab that is not a part at all.
 *
 * ⛔ Song is not a part and never becomes one — it is an *arrangement* of the
 * five. Mapping it to a `Part` here would send it through `session.generate`,
 * which fills the single pattern slot the editors draw from; a song has to go
 * through `useSong` instead. The `null` is what routes it there.
 */
const TAB_PART: Record<GeneratorTab, Part | null> = {
  drums: 'drums',
  melody: 'melody',
  counter: 'counter',
  bass: 'bass',
  chords: 'chords',
  song: null,
};

/**
 * Centre stage: the tab strip over the editor for the part it names.
 *
 * Drums draws in `DrumGrid`; the four melodic parts draw in the piano roll
 * (TASK-041); Song draws in `SongTimeline` (TASK-063A / TASK-063B).
 *
 * ⛔ **The editor is shown only when the pattern in hand is the tab's own
 * part.** `session.pattern` is one slot, so generating a melody replaces the
 * drums that were there — and drawing whatever is in the slot under whichever
 * tab is open would put a melody's notes in the drum grid's lanes. Switching to
 * a tab whose part is not loaded shows the "ready, hit Generate" state, which is
 * true rather than merely blank.
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

  const song = useSong((s) => s.song);
  const songGenerating = useSong((s) => s.generating);
  const songError = useSong((s) => s.error);
  const buildSong = useSong((s) => s.generate);
  const seed = useSession((s) => s.seed);
  const pins = useSession((s) => s.pins);
  const mood = useSession((s) => s.mood);

  // Song Mode reads the same seed, pins and mood the chips already hold, so the
  // arrangement is placed in the session the producer set up rather than in one
  // of its own.
  const generateSong = () => {
    if (!selectedId) return;
    return buildSong({ styleId: selectedId, seed, pins, mood });
  };

  const selected = roster.find((entry) => entry.id === selectedId) ?? null;
  const part = TAB_PART[activeTab];

  // The pattern in hand belongs to this tab, so it is this tab's to draw.
  const showing = part !== null && pattern?.part === part ? pattern : null;

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
          {/* Ordered so the compiler can narrow `part`: past the `null` and the
              `drums` arms, what is left is exactly a melodic part, which is what
              the roll accepts. A conjunction here instead would leave `part`
              still possibly `'drums'` on the roll's branch. */}
          {part === null ? (
            // Song is not a part — it is an arrangement of the five, so it
            // draws its own surface rather than an editor for a `Part`
            // (TASK-063A / TASK-063B).
            song === null ? (
              <div className="stage__empty">
                <h2>{t('tabs.song')}</h2>
                <p>
                  {selected
                    ? t('song.readyBody', { name: selected.name })
                    : t('stage.emptyBody')}
                </p>
              </div>
            ) : (
              <SongTimeline song={song} />
            )
          ) : showing === null ? (
            <div className="stage__empty">
              <h2>{t('stage.emptyTitle')}</h2>
              <p>
                {selected
                  ? t('stage.readyBody', { name: selected.name })
                  : t('stage.emptyBody')}
              </p>
            </div>
          ) : part === 'drums' ? (
            <DrumGrid pattern={showing} playhead={playhead} />
          ) : (
            <PianoRoll pattern={showing} part={part} playhead={playhead} />
          )}
        </GenFx>

        {/* One positioned column above the bottom-right corner. `.genfx` is
            `inset: 0` over the whole body, so a static sibling of it paints
            underneath — which is where the error message used to go. */}
        <div className="stage__bottom">
          {/* The error sits beside the control that caused it rather than in a
              toast that has to be chased across the screen. */}
          {(error ?? songError) && (
            <p className="stage__error" role="alert">
              {error ?? songError}
            </p>
          )}

          {/* Beside Generate rather than beside the chips it is about: the
              right rail collapses under 1440px and behind K, and this must
              not. */}
          <SessionSwitchPrompt />

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

            {/* ⛔ Song generates a whole arrangement rather than a part, so it
                cannot go through `session.generate` — that fills the one pattern
                slot the roll draws from, and a song is an arrangement of five.
                The button was previously disabled here and said "a later
                phase"; it now does the thing it names. */}
            <button
              type="button"
              className="btn-generate"
              onClick={() => (part !== null ? void generate(part) : void generateSong())}
              disabled={!selectedId || generating || songGenerating}
            >
              {generating || songGenerating ? t('stage.generating') : t('stage.generate')}
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}
