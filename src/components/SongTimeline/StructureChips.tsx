/**
 * The structure chip row and its model-structure picker (TASK-070).
 *
 * Two things in one strip, because they are two views of the same fact:
 *
 * - **The chips** name the form the song on screen actually has, in playing
 *   order, so a producer can read the arrangement without scrolling it.
 * - **The picker** offers the forms the *artist writes*, so the next generation
 *   can be asked for one of them by name instead of re-rolled until it turns up.
 *
 * ⛔ **Only forms the model authored, never a free-text shape.** The whole claim
 * of Song Mode is artist-accuracy, and a picker that let somebody assemble
 * `intro → outro → bridge` would be the generator handing them a song shape
 * nobody researched — which is the same reason `_defaults` was left in place for
 * the two genres whose research states no form.
 */

import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import type { Song } from '../../lib/ipc-types';

type Props = {
  song: Song;
  styleId: string | null;
  /** Which form the next generation should use; `null` lets the artist choose. */
  structure: number | null;
  onPick: (index: number | null) => void;
};

export function StructureChips({ song, styleId, structure, onPick }: Props) {
  const { t } = useTranslation();

  // ⛔ **The artist is stored *with* the forms, and the forms are read back
  // through it.** Clearing them in the effect would be a render where the
  // previous artist's song shapes are offered under the new artist's name —
  // the readout-that-lies failure the roster already guards against, and worse
  // here because a picked index would then mean a different form. Keying the
  // answer to what it is an answer *about* closes that window entirely instead
  // of narrowing it to one frame.
  const [loaded, setLoaded] = useState<{ styleId: string | null; forms: string[][] }>({
    styleId: null,
    forms: [],
  });
  const forms = loaded.styleId === styleId ? loaded.forms : [];

  useEffect(() => {
    if (!styleId) return;
    let live = true;
    void invoke<{ structures: string[][] }>('song_structures', { styleId })
      .then((reply) => {
        if (live) setLoaded({ styleId, forms: reply.structures });
      })
      .catch(() => {
        // A model with no `arrangement` block cannot be arranged at all, and
        // the Generate button already says so. An empty picker is the honest
        // readout here rather than a second error in the same view.
        if (live) setLoaded({ styleId, forms: [] });
      });
    return () => {
      live = false;
    };
  }, [styleId]);

  return (
    <div className="song__structure" data-testid="song-structure">
      <ol className="song__structure-chips" aria-label={t('song.structure')}>
        {song.sections.map((section, index) => (
          <li key={`${section.type}-${index}`} className="song__structure-chip">
            {t(`song.kind.${section.type}`)}
          </li>
        ))}
      </ol>

      {forms.length > 1 && (
        <label className="song__structure-pick">
          <span className="song__structure-pick-label">{t('song.form')}</span>
          <select
            value={structure === null ? '' : String(structure)}
            onChange={(event) =>
              onPick(event.target.value === '' ? null : Number(event.target.value))
            }
          >
            {/* ⛔ First, and it is the default: absence means "the artist
                chooses", sampled from the weights the model authored. That is
                the same meaning absence carries for every pin in this app, and
                it is what makes two generations differ. */}
            <option value="">{t('song.formAny')}</option>
            {forms.map((sections, index) => (
              <option key={index} value={index}>
                {sections.map((name) => t(`song.kind.${chipKey(name)}`)).join(' · ')}
              </option>
            ))}
          </select>
        </label>
      )}
    </div>
  );
}

/**
 * The catalog key for an authored section name.
 *
 * ⚠ The dataset spells a hook `hook` or `chorus` and a pre-chorus `prechorus`
 * or `pre-chorus`; the wire type spells them `hook` and `preChorus`. This is the
 * one place the two vocabularies meet, and an unknown name falls through as
 * itself so a new section kind shows its raw name rather than a blank chip.
 */
function chipKey(name: string): string {
  switch (name.trim().toLowerCase()) {
    case 'chorus':
    case 'hook':
      return 'hook';
    case 'prechorus':
    case 'pre-chorus':
      return 'preChorus';
    default:
      return name.trim().toLowerCase();
  }
}
