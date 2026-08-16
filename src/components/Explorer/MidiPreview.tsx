import { useTranslation } from 'react-i18next';
import { GraduationCap, Pause, Play, Repeat, Split, Square } from 'lucide-react';

import { formatSeconds, useExplorer } from '../../state/explorer';
import { useSession } from '../../state/session';
import { useVariations } from '../../state/variations';
import type { SplitPart } from '../../lib/ipc-types';

/**
 * What a file was found to contain: one row per part, with its reason.
 *
 * ⛔⛔ **ONE component for both roads, because the promise is that they read the
 * same.** Mike: *"can you ensure that you can drag the audio/midi of any sample
 * into any of the generators and it will split them all the same exact way?"* —
 * both produce `SplitPart[]`, both are drawn here, and a new `SplitReason`, an
 * extra column or a reworded heading lands in both panels or in neither. Drawn
 * twice, it would land in one and silently not the other, which is exactly the
 * drift `engine::formats` and `lib/dnd.ts` were each written to stop.
 *
 * ⛔ **Every row carries its reason, and the reasons are not equal.** Channel 10
 * is a fact the file states. "Lowest voice" is a measurement. "Split by pitch"
 * and "the clearest line up top" are heuristics applied when there was nothing
 * else to go on — and they say so, because a producer who is told *why* can tell
 * whether to trust it.
 */
export function SplitRows({ parts }: { parts: SplitPart[] }) {
  const { t } = useTranslation();
  return (
    <ul className="midi__list">
      {parts.map((split) => (
        // ⚠ Keyed on the part: a split never returns the same part twice, and
        // the index would reorder under React if the analysis were re-run.
        <li key={split.part} className="midi__row">
          <span className="midi__part">{t(`tabs.${split.part}`)}</span>
          {/* The bare number, with the unit in the label. */}
          <span className="midi__notes" title={t('explorer.midiNotes')}>
            {split.notes}
          </span>
          {/* ⛔ The reason, in the producer's language rather than an enum. */}
          <span className="midi__reason" title={t(`explorer.splitReason.${split.reason}`)}>
            {t(`explorer.splitReason.${split.reason}`)}
          </span>
        </li>
      ))}
    </ul>
  );
}

/**
 * What a selected `.mid` was found to contain, and where it would go.
 *
 * ⛔⛔ **It says what it detected BEFORE anything is imported**, which is the
 * rule TASK-058D sets for the audio path and which applies just as hard here:
 * *"the UI names what it detected … and never presents a guess as a
 * transcription"*, so that *"a wrong guess is one click to redirect rather than a
 * silent mis-file."* A Split button that only revealed its answer after you
 * pressed it would be exactly the shape that rule forbids.
 *
 * ⛔ **Every row carries its reason, and the reasons are not equal.** Channel 10
 * is a fact the file states. "Lowest voice" is a measurement. "Split by pitch" is
 * a heuristic applied only when the file had nothing else to go on — and it says
 * so, because a producer who is told *why* can tell whether to trust it.
 *
 * ⚠ **This is not the audio path.** Extracting notes from a `.wav` is TASK-058D
 * and is a different, harder problem — see the roadmap for what is and is not
 * possible there without the ML this project bans.
 */
export function MidiPreview() {
  const { t } = useTranslation();
  const midiSplit = useExplorer((s) => s.midiSplit);
  const error = useExplorer((s) => s.error);
  const importSplit = useSession((s) => s.importSplit);
  const audition = useExplorer((s) => s.midiAudition);
  const auditionMidi = useExplorer((s) => s.auditionMidi);
  // ⚠ **The scalars, not the whole `position` object.** It is republished at
  // 30 Hz for as long as anything is sounding, and subscribing to the object
  // re-renders this panel — the split rows and four icon buttons — thirty times
  // a second. `seconds` genuinely does change that fast, so the readout that
  // needs it is its own child; nothing else here depends on it.
  const playing = useExplorer((s) => s.position.playing);
  const looping = useExplorer((s) => s.position.looping);
  const play = useExplorer((s) => s.play);
  const pause = useExplorer((s) => s.pause);
  const stop = useExplorer((s) => s.stop);
  const toggleLoop = useExplorer((s) => s.toggleLoop);
  // The file the split belongs to, which is the key it is kept under.
  const selected = useExplorer((s) => s.selected);
  const keepFile = useVariations((s) => s.keepFile);
  // ⚠ **Subscribed to the one entry, not to the map.** Keeping any file
  // republishes `keptFiles`, and this panel redraws its split rows and five
  // buttons — the question here is only ever about the file on screen.
  const keptHere = useVariations((s) =>
    selected === null ? false : s.keptFiles[selected] !== undefined,
  );

  // ⚠ **Reachable, and in two ways.** `PreviewPlayer` shows this the moment a
  // `.mid` is selected — before the split has answered, and for good if it never
  // does. ⛔ Silent on a failure rather than reading "Reading…" forever: the
  // panel already renders the error underneath, and a spinner over a request that
  // died is the readout-that-lies failure in miniature.
  if (midiSplit === null) {
    return error !== null ? null : (
      <p className="browser__hint preview__idle">{t('explorer.decoding')}</p>
    );
  }

  return (
    <div className="midi">
      {/* ⚠ **No count in the sentence, deliberately.** "1 part"/"3 parts" needs
          i18next plural forms in eighteen catalogs for a heading nobody reads
          twice — and the counts that matter are in the rows below. Grammar this
          panel does not need is grammar that can go wrong in a language nobody
          on this project reads. */}
      <p className="midi__title">{t('explorer.midiFound')}</p>

      <SplitRows parts={midiSplit} />

      {/* ⛔⛔ **Hearing the file, not just reading what is in it** (TASK-160).
          Mike: *".mid files … have its own sound like Ableton does that can play
          the .mid file"*.
          ▶ **Through a neutral built-in instrument**, which is the half the
          roadmap left open and then answered: *"it sounds the same whatever
          artist happens to be selected"*. A producer asking what the notes are
          must not get a different answer on Tuesday because the roster moved.
          ⚠ **It never touches the transport.** `preview_*` is the audition voice
          — the same one a `.wav` plays through — and `Schedule` is the project's.
          That is TASK-058B's rule for this panel and it is structural rather
          than a promise: the two have never shared a playhead. */}
      <div className="preview__bar midi__transport">
        <button
          type="button"
          className="btn-ghost preview__button"
          // ⚠ **`transport.*`, not a second set of strings**, the reason
          // `PreviewPlayer` gives: Play, Pause, Stop and Loop already exist in
          // all eighteen locales and mean the same thing here.
          aria-label={playing ? t('transport.pause') : t('transport.play')}
          onClick={() => {
            if (playing) {
              void pause();
              return;
            }
            // ⛔ **Rendered on the first press, not on selection.** Building the
            // audio for a file is the slow half, and doing it as the producer
            // walks a folder with ↓ would render every `.mid` they step past.
            // Once it is loaded, Play is Play — a second press must not
            // re-render it.
            //
            // ⛔⛔ **Play only if the render actually landed.** `auditionMidi`
            // never rejects — it swallows a failure into `error` and returns
            // early when the selection moved under it — so a bare `.then(play)`
            // fired either way. The audition voice is shared with sample
            // playback, so on a file that failed to render the producer pressed
            // Play on a `.mid` and heard **the last `.wav` they auditioned**,
            // under the MIDI file's name.
            if (audition === null) {
              void auditionMidi().then(() => {
                if (useExplorer.getState().midiAudition !== null) void play();
              });
              return;
            }
            void play();
          }}
        >
          {playing ? (
            <Pause size={13} aria-hidden="true" />
          ) : (
            <Play size={13} aria-hidden="true" />
          )}
        </button>

        <button
          type="button"
          className="btn-ghost preview__button"
          aria-label={t('transport.stop')}
          disabled={audition === null}
          onClick={() => void stop()}
        >
          <Square size={12} aria-hidden="true" />
        </button>

        <button
          type="button"
          className="btn-ghost btn-toggle preview__button"
          aria-label={t('transport.loop')}
          aria-pressed={looping}
          data-on={looping}
          disabled={audition === null}
          onClick={() => void toggleLoop()}
        >
          <Repeat size={13} aria-hidden="true" />
        </button>

        {/* ⚠ Silent until something is loaded rather than showing `0:00 / 0:00`,
            which would read as a file with no notes in it. */}
        {audition !== null && <Elapsed />}
      </div>

      {/* ⚠ **Reported rather than cut in silence** — the rule the truncated
          folder listing follows. A producer who does not hear the end of a long
          arrangement and is told nothing concludes the audition is broken. */}
      {audition?.clipped === true && (
        <p className="browser__hint">{t('explorer.midiAuditionClipped')}</p>
      )}

      <button
        type="button"
        className="btn-ghost midi__apply"
        onClick={() => importSplit(midiSplit)}
      >
        <Split size={12} aria-hidden="true" />
        {t('explorer.midiApply')}
      </button>

      {/* ⛔⛔ **The gesture TASK-040T was waiting on.** The fit, its
          anti-collapse rules, the SMF reader and the keep/train loop have all
          shipped since 2026-08-09 — the reader was reachable only over the
          bridge, so a producer could train on their own *generations* and not on
          their own *files*. This is that door.

          ⚠ **It keeps the split, not the file**, which is what makes a file and
          a generation one measurement rather than two: `StyleEditor` regenerates
          each kept take into a `Pattern` and hands the set to
          `engine::fit`, and these are already that shape. Its own comment says
          so — *"a `.mid` dragged in from the browser will arrive as a `Pattern`
          too, so a model fitted from files cannot drift from one fitted from
          generations."*

          ⚠ **A toggle rather than an Add button**, so keeping is undoable in the
          place it was done. Pressed state is `aria-pressed` and `data-on`, the
          same treatment the loop buttons wear. */}
      {/* ⚠ **Not offered on a file with nothing in it.** An empty split is a real
          answer — `explorer_midi_split` returns one for a file it can read and
          finds no notes in — and keeping it would put a lit toggle next to zero
          patterns, which is a control claiming to have done something. */}
      {selected !== null && midiSplit.length > 0 && (
        <button
          type="button"
          className="btn-ghost btn-toggle midi__train"
          aria-pressed={keptHere}
          data-on={keptHere}
          onClick={() =>
            keepFile(
              { path: selected, patterns: midiSplit.map((split) => split.pattern) },
              !keptHere,
            )
          }
        >
          <GraduationCap size={12} aria-hidden="true" />
          {t('explorer.midiTrain')}
        </button>
      )}

      {/* ⚠ Named rather than implied: a producer who does not want the split can
          still drag the file onto one generator, and this is where they learn
          that is an option. */}
      <p className="browser__hint">{t('explorer.midiDragHint')}</p>
    </div>
  );
}

/**
 * The audition's playhead, as a time out of a total.
 *
 * ⛔ **Its own component so its 30 Hz subscription is its own.** `seconds` really
 * does change thirty times a second while a file sounds, and `MidiPreview` above
 * draws the split rows and four buttons — none of which depend on it. Read there,
 * the whole panel re-rendered at the poll rate to move one line of text.
 */
function Elapsed() {
  const seconds = useExplorer((s) => s.position.seconds);
  const total = useExplorer((s) => s.position.total);
  return (
    <span className="preview__time">
      {formatSeconds(seconds)} / {formatSeconds(total)}
    </span>
  );
}
