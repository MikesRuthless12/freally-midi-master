import { useTranslation } from 'react-i18next';
import { Split } from 'lucide-react';

import { useExplorer } from '../../state/explorer';
import { useSession } from '../../state/session';

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

      <ul className="midi__list">
        {midiSplit.map((split) => (
          // ⚠ Keyed on the part: `split` never returns the same part twice, and
          // the index would reorder under React if the analysis were re-run.
          <li key={split.part} className="midi__row">
            <span className="midi__part">{t(`tabs.${split.part}`)}</span>
            {/* The bare number, with the unit in the label — see above. */}
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

      <button
        type="button"
        className="btn-ghost midi__apply"
        onClick={() => importSplit(midiSplit)}
      >
        <Split size={12} aria-hidden="true" />
        {t('explorer.midiApply')}
      </button>

      {/* ⚠ Named rather than implied: a producer who does not want the split can
          still drag the file onto one generator, and this is where they learn
          that is an option. */}
      <p className="browser__hint">{t('explorer.midiDragHint')}</p>
    </div>
  );
}
