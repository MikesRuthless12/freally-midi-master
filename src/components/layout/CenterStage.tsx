import { Fragment, useMemo, useState } from 'react';
import { AudioWaveform, Drum, ListMusic, Music2, Piano, Waves, X } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { GENERATOR_TABS, useUi, type GeneratorTab } from '../../state/ui';
import type { Lane, Part } from '../../lib/ipc-types';
import {
  armCurrentPattern,
  BAR_CHOICES,
  GENERATED_PARTS,
  TAB_PART,
  useSession,
} from '../../state/session';
import { DrumGrid } from '../DrumGrid/DrumGrid';
import { PianoRoll } from '../PianoRoll/PianoRoll';
import { SongTimeline } from '../SongTimeline/SongTimeline';
import { useSong } from '../../state/song';
import { columnDensity } from '../DrumGrid/cells';
import { sectionDensity } from '../SongTimeline/sketch';
import { GenFx } from '../GenFx/GenFx';
import { SeedChip } from '../SeedChip/SeedChip';
import { SessionSwitchPrompt } from '../SessionChips/SessionChips';
import { TransportControls } from './TransportBar';
import { VariationNav } from '../VariationNav';
import { MIDI_TYPE, SAMPLE_TYPE, droppedMidi, droppedSample } from '../../lib/dnd';
import { useExplorer } from '../../state/explorer';
import { useKit } from '../../state/kit';
import { PadGrid } from '../Kit/PadGrid';
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

/*
 * ⚠ **`PartToggles` is gone, and its rule moved with it** (2026-08-09).
 *
 * It was a row of six buttons beside the tablist carrying the same six words as
 * the tabs, so working out which one silenced Drums meant reading "Drums" twice
 * and matching them up. Mike asked for the switch to live on the tab itself:
 * *"it should be a button … in the top right side of the tab, so that way you
 * can mute it by just clicking the 'Green' dot and it will make it 'Red'."*
 *
 * ⛔ The constraint that shaped the old design still holds and is honoured in
 * `GeneratorTabs`: a `role="tab"` **may not contain a second button** — nesting
 * interactive controls breaks the keyboard model and every screen reader — so
 * the dot is a *sibling* of the tab inside a slot wrapper, never a child of it.
 *
 * ⚠ And the Song-tab rule came with it: `TAB_PART.song` is `null`, so re-arming
 * from the Song tab *disarms* the whole arrangement.
 */

function GeneratorTabs() {
  const { t } = useTranslation();
  const activeTab = useUi((s) => s.activeTab);
  const setActiveTab = useUi((s) => s.setActiveTab);
  const patterns = useSession((s) => s.patterns);
  const partsOff = useUi((s) => s.partsOff);
  const togglePart = useUi((s) => s.togglePart);
  const clearPart = useSession((s) => s.clearPart);
  const importMidi = useSession((s) => s.importMidi);
  const importSong = useSong((s) => s.importSong);
  const [over, setOver] = useState<string | null>(null);

  return (
    <div className="tabs" role="tablist" aria-label={t('tabs.group')}>
      {GENERATOR_TABS.map((tab, index) => {
        const Icon = TAB_ICONS[tab];
        const selected = tab === activeTab;
        // ⛔ **The mute dot only exists once the part does.** A dot on a tab with
        // no clip behind it would offer to silence something that is not making
        // a sound — and `partsOff` would then carry a part that was never
        // generated. `song` is not a generated part and has none.
        const part = GENERATED_PARTS.find((candidate) => candidate === (tab as Part));
        const hasClip = part !== undefined && patterns[part] != null;
        const on = part !== undefined && !partsOff.includes(part);

        return (
          <div
            key={tab}
            className="tab-slot"
            data-midi-over={over === tab}
            // ⛔⛔ **A generator is a drop target for a `.mid`** — Mike,
            // 2026-08-10: *"be able to drag the file to a generator."* The tab
            // rather than the editor below it, because the tab is the one place
            // that names the part while every part is visible at once: dropping
            // on the roll could only ever mean "the part already showing", which
            // makes the gesture a two-step (switch tab, then drag).
            //
            // ⛔ **`MIDI_TYPE`, so a sample cannot land here and a `.mid` cannot
            // land on a pad.** `FileTree` puts the two kinds on different MIME
            // types precisely so each drop target refuses the other *before* the
            // producer releases — the cursor says no rather than the app
            // erroring afterwards.
            //
            // ⚠ Song takes no drop: it is an arrangement of clips, not a clip,
            // and `openClip` has no part to open one into.
            onDragOver={(event) => {
              if (!event.dataTransfer.types.includes(MIDI_TYPE)) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = 'copy';
              if (over !== tab) setOver(tab);
            }}
            onDragLeave={() => setOver((held) => (held === tab ? null : held))}
            onDrop={(event) => {
              setOver(null);
              const path = droppedMidi(event.dataTransfer);
              if (path === null) return;
              event.preventDefault();
              // ⛔⛔ **Song takes the whole file as an arrangement** (TASK-058D).
              // Mike, 2026-08-10: *"could you put the midi for the entire song
              // into the 'Song' tab and allow them to pick which parts they want
              // for the generators."* The other five tabs still take the file
              // into that one generator, which is the "I know what this is"
              // shortcut; Song is the "show me what is in it" route.
              if (part === undefined) {
                void importSong(path);
                return;
              }
              void importMidi(path, part);
            }}
          >
            <button
              type="button"
              role="tab"
              id={`tab-${tab}`}
              aria-selected={selected}
              aria-controls="generator-panel"
              tabIndex={selected ? 0 : -1}
              className="tab"
              // FR-018's "tooltips show keys everywhere". ⚠ The digit comes from
              // the tab's *position*, the same way `App.tsx` binds it — a
              // hardcoded number here would go wrong the day a generator is
              // added, and it would go wrong silently.
              title={`${t(`tabs.${tab}`)} — ${index + 1}`}
              onClick={() => setActiveTab(tab)}
            >
              <Icon size={16} aria-hidden="true" />
              {t(`tabs.${tab}`)}
            </button>

            {/* ⛔⛔ **The mute switch, on the tab it belongs to** — Mike,
                2026-08-09: *"the 'Drums' 'Melody' etc. for the muting should not
                be at the top to the left of being able to select a historical
                generation, it should be a button … in the top right side of the
                tab, so that way you can mute it by just clicking the 'Green' dot
                and it will make it 'Red'."* It was a second row of the same six
                words, which meant reading "Drums" twice to work out which one
                silenced it. On the tab there is nothing to match up.
                ⚠ A sibling of the tab, not a child: a button inside a button is
                not something HTML allows, and the click would go to whichever
                the browser preferred. */}
            {hasClip && part !== undefined && (
              <button
                type="button"
                className="tab-mute"
                data-on={on}
                aria-pressed={!on}
                aria-label={t(on ? 'kit.muteLane' : 'kit.unmuteLane', {
                  lane: t(`tabs.${tab}`),
                })}
                title={t(on ? 'kit.muteLane' : 'kit.unmuteLane', { lane: t(`tabs.${tab}`) })}
                onClick={() => {
                  togglePart(part);
                  // ⛔ Re-arm here rather than from the store: the schedule holds
                  // one merged clip, so a switch that changed only the UI would
                  // leave Play sounding the previous combination — a control that
                  // looks like it did something and did not.
                  //
                  // ⛔⛔ **Never on the Song tab.** `TAB_PART.song` is `null`, so
                  // `armCurrentPattern` falls through to *disarm* — generate a
                  // song, hit a part switch, and the whole arrangement goes
                  // silent with the timeline still on screen and Play still lit.
                  if (activeTab !== 'song') armCurrentPattern();
                }}
              />
            )}

            {/* ⛔⛔ **Clear THIS generator, from its own tab** — Mike,
                2026-08-11: *"there should be a 'C' or an 'X' in the tab for the
                generators, so that way you can clear just the single
                generator"* — *"just like the drum pad lanes."*

                ▶ **It replaces the `Clear` half of the footer pair**, which sat
                beside `Clear all` with no styling between them and read as one
                control saying *"Clear Clear all"*. Splitting them by *place*
                rather than by label is the fix he described: the one that clears
                a part lives on the part, and the footer keeps only the one that
                clears everything.

                ⚠ **Only once there is something to clear**, the same rule the
                mute dot follows — an ✕ on an empty generator is a control that
                can only do nothing.

                ⚠ A sibling of the tab, never a child: `role="tab"` may not
                contain a second button, which is the constraint this file's
                header already records for the dot. */}
            {hasClip && part !== undefined && (
              <button
                type="button"
                className="tab-clear"
                aria-label={t('stage.clearOne', { part: t(`tabs.${tab}`) })}
                title={t('stage.clearOne', { part: t(`tabs.${tab}`) })}
                onClick={() => {
                  clearPart(part);
                  // Re-armed for the same reason the mute switch is: the
                  // schedule holds one merged clip, so clearing a part without
                  // re-arming leaves Play sounding the one that is gone.
                  if (activeTab !== 'song') armCurrentPattern();
                }}
              >
                <X size={11} aria-hidden="true" />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

/**
 * The transport on the first row, the tab strip on the second (TASK-127 /
 * TASK-138).
 *
 * ⛔ **Two rows, transport above tabs** — Mike, 2026-08-09: *"move these buttons
 * to the first row and have the tabs on the second row above the piano roll."*
 *
 * ⚠ **This supersedes the 2026-08-06 arrangement**, which put both on one row
 * with the transport *"to the right of the generators tabs."* That request is
 * still honoured in substance — the transport is at the top of the app and plays
 * whichever generator is showing — and the reason it changed is the reason it
 * was cramped: a row holding six tabs, five switches, the take history and three
 * transport buttons has to scroll the tabs at any window narrower than the
 * layout, which is exactly when a producer most needs to see them.
 *
 * The switches stay with the transport because they are what Play acts on, and
 * the take history stays beside it because choosing a take is the same gesture
 * as playing one.
 */
function StageHeader() {
  return (
    <div className="stage__header">
      <div className="stage__header-controls">
        {/* ⚠ The part switches moved onto the tabs on 2026-08-09 — see `tab-mute`
            in `GeneratorTabs`. They were a second row of the same six words. */}
        {/* ⛔ **Beside the transport, because the two are the same gesture.**
            A producer choosing a take alternates between Generate and stepping
            back to the one before it, and putting the history in a rail would
            make "go back one" a trip across the window. */}
        <VariationNav />

        <TransportControls />
      </div>

      {/* ⛔ **The pads, under the transport** (TASK-054). Mike, 2026-08-09: *"put
          the drum kit squares below those buttons you just centered 8 going
          across the length of the piano roll."* Here rather than in the right
          rail because that is the whole point of the task — a rail row 280px
          wide is what nobody could find. */}
      <PadGrid />

      {/* Directly above the editor it labels — a tab strip separated from its
          own content by a row of controls reads as belonging to the controls. */}
      <GeneratorTabs />
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
  const clearAll = useSession((s) => s.clearAll);
  const playhead = useSession((s) => s.playhead);
  const dropOn = useExplorer((s) => s.dropOn);
  const refreshKit = useKit((s) => s.refresh);

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

  // ⛔ **A melodic part's id *is* its lane id** — `melody`, `counter`, `bass`,
  // `chords` — which is what makes the editor able to name its own target
  // without a lookup. `ExplorerPanel` leans on the same identity for `Ctrl`+←/→.
  const dropTarget: Lane | null = part === null || part === 'drums' ? null : (part as Lane);
  const [sampleOver, setSampleOver] = useState(false);

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
      <StageHeader />

      <div
        className="stage__body"
        role="tabpanel"
        id="generator-panel"
        aria-labelledby={`tab-${activeTab}`}
        data-sample-over={sampleOver}
        // ⛔⛔ **THE EDITOR IS A DROP TARGET FOR A SAMPLE** — Mike, 2026-08-12:
        // *"you should be able to drag a sample to the actual generator's piano
        // roll and it should drop the sample on there for that specific
        // generator as well … so that way you don't have to drag all the way to
        // the other side and try to land on a small button."* The small button
        // is `pad__use` in the right rail; the roll is the largest surface on
        // screen and it is already the thing the producer is looking at.
        //
        // ⛔⛔ **Melodic only. The drum grid refuses**, on his instruction in the
        // same breath: *"the drum generator, you should only be able to drag a
        // sample to the drum pads themselves."* Which is right — a drum kit has
        // eight lanes showing at once and the panel could only ever guess one of
        // them, whereas a melodic generator *is* one lane, so there is nothing
        // to guess. `dropTarget` is `null` on Drums and on Song, and a `null`
        // target never calls `preventDefault`, so the cursor says no.
        //
        // ⚠ `SAMPLE_TYPE`, so the `.mid` that the tabs above take cannot land
        // here instead — the two gestures mean completely different things and
        // `FileTree` puts them on different MIME types precisely so each target
        // can refuse the other before the producer lets go.
        onDragOver={(event) => {
          if (dropTarget === null || !event.dataTransfer.types.includes(SAMPLE_TYPE)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = 'copy';
          if (!sampleOver) setSampleOver(true);
        }}
        // ⚠ **`relatedTarget`, because `dragleave` bubbles from every child.**
        // Without the containment test, dragging a sample *across* the roll fires
        // a leave at each element boundary crossed and the next `dragover` sets
        // it back — two full re-renders of the stage per crossing, and a drop
        // outline that flickers all the way in.
        onDragLeave={(event) => {
          if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
          setSampleOver(false);
        }}
        onDrop={(event) => {
          setSampleOver(false);
          if (dropTarget === null) return;
          const path = droppedSample(event.dataTransfer);
          if (path === null) return;
          event.preventDefault();
          // Refreshed after, because the Kit rail's row for this lane is the
          // only thing on screen that names what landed.
          void dropOn(dropTarget, path).then(() => refreshKit());
        }}
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

          {/* ⛔⛔ **`4 / 8 Bars`, with the live one filled in** — Mike,
              2026-08-11: *"the button should say '4/8 Bars' and it should switch
              colors or it should be a toggle when it is 4 or 8 bars so that way
              you know which one you are on, and it won't let me switch back to 4
              after switching to 8 bars."*

              ▶ **`.chip__option` had no CSS whatsoever**, which is the whole
              report: zero padding made the pair render as the single word `48`,
              so `4` was a seven-pixel target sharing an edge with `8` and there
              was no pressed state to say which had won. Nothing was wrong with
              `setBars` — `session.test.ts` round-trips 4→8→4 — the switch back
              was simply unhittable and, once hit, invisible.

              ⚠ **Two buttons and not one toggle.** He offered either ("or it
              should be a toggle"); a toggle costs a press to *discover* which
              state it is in, and these are two named lengths, not on/off. */}
          <span
            className="chip chip--mono stage__bars"
            role="group"
            aria-label={t('stage.barsLabel')}
          >
            {BAR_CHOICES.map((choice, index) => (
              <Fragment key={choice}>
                {index > 0 && (
                  <span className="chip__slash" aria-hidden="true">
                    /
                  </span>
                )}
                <button
                  type="button"
                  className="chip__option"
                  aria-pressed={bars === choice}
                  title={`${choice} ${t('stage.bars')}`}
                  onClick={() => setBars(choice)}
                >
                  {choice}
                </button>
              </Fragment>
            ))}
            {/* ⚠ A span, because the chip is `display: inline-flex` now and a
                bare text node's leading space would collapse — `8bars`. */}
            <span className="chip__unit">{t('stage.bars')}</span>
          </span>

          {/* Clear this tab's own part, and all five (TASK-121). Hidden on
                Song, which is not a part — its clips are cleared from the
                timeline's own controls. Disabled rather than absent when the
                slot is empty, so the control does not move under the pointer as
                parts fill in. */}
          {part !== null && (
            // ⛔ **One button, and it clears everything** — Mike, 2026-08-11:
            // *"the button should say 'Clear All' not 'Clear Clear All'"*, and
            // *"it should clear all generators."* The per-part half moved onto
            // the generator tabs, where it says which part it means by *being
            // there*; the two of them side by side with no styling between read
            // as one nonsense label.
            <button
              type="button"
              className="btn-ghost stage__clear"
              onClick={clearAll}
              disabled={Object.keys(patterns).length === 0}
            >
              {t('stage.clearAll')}
            </button>
          )}

          {/* Fill all five from one seed (TASK-120). Not offered on Song,
                which already generates every part as an arrangement. */}
          {part !== null && (
            <button
              type="button"
              className="btn-generate btn-generate--secondary"
              // FR-018: the key is on the control, so a producer learns it
              // where they already are rather than from a panel they have to
              // know exists.
              title={`${t('stage.generateAll')} — Shift + G`}
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
            title={part !== null ? `${t('stage.generate')} — G` : t('stage.generate')}
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
