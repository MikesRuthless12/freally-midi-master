import { useTranslation } from 'react-i18next';
import { Music4, Split, X } from 'lucide-react';

import { useExplorer } from '../../state/explorer';
import { sessionGrid, useSession } from '../../state/session';
import { SplitRows } from './MidiPreview';

/**
 * Reading a sample's notes out, and saying what was found (TASK-058D).
 *
 * ⛔⛔ **It says what it detected BEFORE anything is imported**, which is the
 * rule TASK-058D sets in as many words: *"the UI names what it detected — 'drum
 * loop', 'bassline', 'progression' — and never presents a guess as a
 * transcription"*, so that *"a wrong guess is one click to redirect rather than a
 * silent mis-file."* `MidiPreview` already draws the same three columns for a
 * `.mid`, and the rows are the same type because both roads end in `SplitPart`.
 *
 * ⛔ **But it is a BUTTON here and a selection there, and the difference is
 * measured.** Splitting a `.mid` is microseconds; reading a `.wav` is a decode,
 * four band filters and two pitch tracks — about two seconds on a forty-second
 * stem. Running that on every arrow-key step through a folder would make the
 * browser unusable, so the producer asks. Everything else about the panel is the
 * same, including that it answers before it commits.
 *
 * ⚠ **The reasons are not equal and the panel does not pretend they are.** "It
 * survived a low-pass at 250 Hz and held a pitch" is close to a fact; "the most
 * clearly periodic line in the lead register" is the weakest thing this module
 * says, because leads, keys, guitars, brass and voices all live in that band.
 * Both carry their own words.
 */
export function AudioNotes() {
  const { t } = useTranslation();
  const selected = useExplorer((s) => s.selected);
  const audioSplit = useExplorer((s) => s.audioSplit);
  const extracting = useExplorer((s) => s.extracting);
  const extractNotes = useExplorer((s) => s.extractNotes);
  const cancelExtract = useExplorer((s) => s.cancelExtract);
  const importSplit = useSession((s) => s.importSplit);

  if (selected === null) return null;

  // ⛔⛔ **Both of these are compared against the SELECTED file, not merely
  // checked for existence.** A read does not have to be of the file the browser
  // is showing: dropping a sample onto a generator tab starts one for a file the
  // producer never clicked. Keyed on existence alone, this panel showed
  // `clap-01.wav`'s waveform above `kick-808.wav`'s parts, with a **Stop
  // reading** button that cancelled the other read and a **Send to generators**
  // button for notes that came from a different sample.
  if (extracting !== null && extracting === selected) {
    return (
      <div className="midi">
        <p className="browser__hint preview__idle">{t('explorer.decoding')}</p>
        {/* ⚠ **Reachable, because the read takes seconds.** `extract::job`'s own
            note is honest about what this does: it releases the producer, not
            the CPU — the worker finishes and its answer is dropped. */}
        <button type="button" className="btn-ghost midi__apply" onClick={() => cancelExtract()}>
          <X size={12} aria-hidden="true" />
          {t('explorer.extractCancel')}
        </button>
      </div>
    );
  }

  if (audioSplit === null || audioSplit.path !== selected) {
    return (
      <button
        type="button"
        className="btn-ghost midi__apply"
        // ⚠ Disabled rather than hidden while another file is being read: the
        // plugin holds one job at a time, so starting a second would abandon the
        // first — and a button that silently does that is worse than one that
        // waits.
        disabled={extracting !== null}
        onClick={() => {
          // ⚠ **The session's tempo goes with the request**, and it is the only
          // thing it is used for: eight bars at 140 and four at 70 are the same
          // waveform, and nothing in the audio can tell them apart. See
          // `extract::grid`.
          void extractNotes(selected, sessionGrid(useSession.getState()));
        }}
      >
        <Music4 size={12} aria-hidden="true" />
        {t('explorer.extract')}
      </button>
    );
  }

  return (
    <div className="midi">
      <p className="midi__title">{t('explorer.audioFound')}</p>

      {/* ⛔ **Reported rather than silently dropped** (TASK-058G). Mike asked for
          *"and leave the vocals alone"* — and a producer who reads an a-cappella
          and sees an empty panel has been told the file is broken. */}
      {audioSplit.found.vocalLeftAlone && (
        <p className="browser__hint">{t('explorer.vocalLeftAlone')}</p>
      )}

      {/* ⛔ The same rows the `.mid` panel draws, from the same component — see
          `SplitRows`. Both roads produce `SplitPart[]`, and the promise is that a
          producer sees the same readout whichever door they came through. */}
      <SplitRows parts={audioSplit.found.parts} />

      {audioSplit.found.parts.length > 0 && (
        <button
          type="button"
          className="btn-ghost midi__apply"
          onClick={() => importSplit(audioSplit.found.parts)}
        >
          <Split size={12} aria-hidden="true" />
          {t('explorer.midiApply')}
        </button>
      )}

      {/* ⚠ Named rather than implied, exactly as the MIDI panel does: a producer
          who wants the file on one generator can drag it there. */}
      <p className="browser__hint">{t('explorer.audioDragHint')}</p>
    </div>
  );
}
