import { useMemo } from 'react';
import { AudioWaveform, Drum, ListMusic, Music2, Piano, Waves } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { GENERATOR_TABS, useUi, type GeneratorTab } from '../../state/ui';
import { BAR_CHOICES, TAB_PART, useSession } from '../../state/session';
import { DrumGrid } from '../DrumGrid/DrumGrid';
import { PianoRoll } from '../PianoRoll/PianoRoll';
import { SongTimeline } from '../SongTimeline/SongTimeline';
import { useSong } from '../../state/song';
import { columnDensity } from '../DrumGrid/cells';
import { sectionDensity } from '../SongTimeline/sketch';
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
 * Centre stage: the tab strip over the editor for the part it names.
 *
 * Drums draws in `DrumGrid`; the four melodic parts draw in the piano roll
 * (TASK-041); Song draws in `SongTimeline` (TASK-063A / TASK-063B).
 *
 * ✅ **Each tab draws its own part's clip, and the other four are still there**
 * (TASK-119). This used to read a single `session.pattern` and show an editor
 * only when that one slot happened to hold the tab's part — which was not a
 * display rule but a symptom: generating a melody genuinely destroyed the drums,
 * and drawing the slot regardless would have put a melody's notes in the drum
 * grid's lanes. A tab whose part has never been generated still shows the
 * "ready, hit Generate" state, which is now true because the part is absent
 * rather than because something else overwrote it.
 */
export function CenterStage() {
  const { t } = useTranslation();
  const activeTab = useUi((s) => s.activeTab);

  const selectedId = useSession((s) => s.selectedId);
  const roster = useSession((s) => s.roster);
  const patterns = useSession((s) => s.patterns);
  const bars = useSession((s) => s.bars);
  const setBars = useSession((s) => s.setBars);
  const generating = useSession((s) => s.generating);
  const error = useSession((s) => s.error);
  const generate = useSession((s) => s.generate);
  const generateAll = useSession((s) => s.generateAll);
  const clearPart = useSession((s) => s.clearPart);
  const clearAll = useSession((s) => s.clearAll);
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

  // This tab's own slot. Absent means nobody has generated it — not that
  // another part took its place.
  const showing = part === null ? null : (patterns[part] ?? null);

  // What the ripple ignites. Recomputed only when the pattern changes, not on
  // every frame — the animation reads this, and it must not cost a render.
  //
  // ⛔ **On the Song tab it is the *sections*, which is what makes the FX
  // cascade section by section (TASK-073).** The ripple already sweeps
  // left→right and lights each column by its own density, so a song only has to
  // say what "a column" means here — one bucket per section, weighted by how
  // much is playing in it. Writing a second animation for Song Mode would have
  // meant a second reduced-motion path as well, and that is the one this
  // project has already had to fix twice.
  const density = useMemo(() => {
    if (activeTab === 'song') return song ? sectionDensity(song) : undefined;
    // ⚠ The tab's own clip, not "whatever was generated last" (TASK-119). With
    // five slots, the latter would ignite the ripple over the drum grid with a
    // bassline's density.
    return showing ? columnDensity(showing, FX_COLUMNS) : undefined;
  }, [activeTab, song, showing]);

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
        <GenFx active={generating || songGenerating} density={density}>
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
            /* ⛔ **`key` is load-bearing, not a list-rendering habit.** Note ids
               are `${startTick}:${pitch}` and `useEditing.selection` is a global
               store nothing clears on a part change — so without a remount the
               same roll instance re-renders with the counter's notes while the
               melody's ids are still selected, and any counter note at a
               matching tick and pitch draws selected. Delete or a Transform then
               edits notes the producer never chose on that tab. Before the five
               slots existed, two parts could not both hold a clip, so the tabs
               were never one click apart. */
            <PianoRoll key={part} pattern={showing} part={part} playhead={playhead} />
          )}
        </GenFx>
      </div>

      {/* ⛔⛔ **Below the editor, in flow — NOT floating over its bottom-right
          corner, which is where this lived until 2026-08-06 and what made the
          velocity lane partly dead to the pointer.** Measured then: the lane
          spanned y 748–843 and this column sat at 776–820, so
          `document.elementFromPoint` over that region answered with a *control*
          and a producer dragging a cap under one moved nothing at all. It was
          invisible for a long time because the column is right-aligned — it
          only reaches a given cap once it grows wide enough to — and it was
          found by adding one small button to the seed chip.

          ⚠ **The paint-order reason it used to be positioned is gone rather
          than ignored.** `.genfx` is `position: absolute; inset: 0` over the
          whole body, so a *static sibling of it* paints underneath — which is
          why `.stage__error` was once invisible and why this column was made
          `position: absolute` to escape. Out here it is no longer a sibling of
          `.genfx` at all, so there is nothing to escape from.

          ⚠ This row now costs the editor real height, and that is the honest
          trade Mike chose: it was costing the same height before, except the
          pixels looked usable. */}
      <div className="stage__bottom">
        {/* The error sits beside the control that caused it rather than in a
              toast that has to be chased across the screen. */}
        {/* ⛔ The error belongs to the tab that is showing. `error ?? songError`
              showed the *session's* error whenever one existed, so a stale
              "trap's 808 is the bassline" from a Bass request on another tab was
              presented as though it were about the song. */}
        {(part === null ? songError : error) && (
          <p className="stage__error" role="alert">
            {part === null ? songError : error}
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

          {/* Clear this tab's own part, and all five (TASK-121). Hidden on
                Song, which is not a part — its clips are cleared from the
                timeline's own controls. Disabled rather than absent when the
                slot is empty, so the control does not move under the pointer as
                parts fill in. */}
          {part !== null && (
            <span className="chip chip--mono" role="group" aria-label={t('stage.clearLabel')}>
              <button
                type="button"
                className="chip__option"
                onClick={() => clearPart(part)}
                disabled={patterns[part] === undefined}
              >
                {t('stage.clear')}
              </button>
              <button
                type="button"
                className="chip__option"
                onClick={clearAll}
                disabled={Object.keys(patterns).length === 0}
              >
                {t('stage.clearAll')}
              </button>
            </span>
          )}

          {/* Fill all five from one seed (TASK-120). Not offered on Song,
                which already generates every part as an arrangement. */}
          {part !== null && (
            <button
              type="button"
              className="btn-generate btn-generate--secondary"
              onClick={() => void generateAll()}
              disabled={!selectedId || generating || songGenerating}
            >
              {t('stage.generateAll')}
            </button>
          )}

          {/* ⛔ Song generates a whole arrangement rather than a part, so it
                cannot go through `session.generate` — that fills the pattern
                slots the roll draws from, and a song is an arrangement of five.
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
    </section>
  );
}
