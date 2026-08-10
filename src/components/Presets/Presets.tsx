import { Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import { useSession, type SavedSession } from '../../state/session';
import { Combo } from '../Combo/Combo';
import './Presets.css';

/**
 * Presets, owned by the plugin rather than by the host (TASK-P13).
 *
 * A preset is the session — artist, seed, bars, pins — under a name, kept
 * outside any one project. That is the whole difference from the state the DAW
 * saves: the project remembers *this song*, a preset carries *any song*.
 *
 * **Mike ruled out CLAP's preset-discovery factory**, which would have put
 * these in the host's own browser. They live here instead, which is why this is
 * a panel and not a manifest.
 *
 * Factory presets are shipped and cannot be deleted; the list marks them so the
 * delete control is absent rather than present-and-failing.
 */
type PresetSummary = {
  id: string;
  name: string;
  factory: boolean;
};

export function Presets() {
  const { t } = useTranslation();
  const applyPreset = useSession((s) => s.applyPreset);

  const [presets, setPresets] = useState<PresetSummary[]>([]);
  const [name, setName] = useState('');
  const [error, setError] = useState<string | null>(null);
  /** Which preset the combobox is showing. Its own state: applying one does not
   *  make it "the session's preset", so nothing else has an opinion about it. */
  const [chosen, setChosen] = useState<string | null>(null);
  const chosenPreset = presets.find((preset) => preset.id === chosen);

  // `.then` rather than `async`/`await`, matching `BugReport`: the state is set
  // from a promise callback rather than from the effect body, which is what
  // `react-hooks/set-state-in-effect` is asking for.
  const fail = (cause: unknown) =>
    setError(cause instanceof Error ? cause.message : String(cause));

  const refresh = () => {
    invoke<PresetSummary[]>('presets_list')
      .then((list) => {
        setPresets(list);
        setError(null);
      })
      .catch(fail);
  };

  useEffect(refresh, []);

  const load = (id: string) => {
    invoke<SavedSession>('preset_load', { id })
      .then((session) => {
        applyPreset(session);
        setError(null);
      })
      .catch(fail);
  };

  const save = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    invoke('preset_save', { name: trimmed })
      .then(() => {
        setName('');
        refresh();
      })
      .catch(fail);
  };

  const remove = (id: string) => {
    invoke('preset_delete', { id }).then(refresh).catch(fail);
  };

  return (
    <div className="presets">
      {/* ⛔ **A combobox rather than a list** — Mike, 2026-08-09: *"the presets
          on the right need to be just a auto-complete combobox like the
          artists/producers/genre."* It is the same control, so it behaves the
          same way: type to filter, or press the arrow for the whole list. A
          rail-height list of presets grows without bound as a producer saves
          them, and it was already the tallest panel in the rail.
          ⚠ **Applying is the change**, so this loads on pick rather than
          needing a second confirm — the same as choosing an artist. */}
      {presets.length > 0 && (
        <div className="presets__pick">
          <Combo
            label={t('sections.presets')}
            options={presets.map((preset) => ({
              id: preset.id,
              name: preset.name,
              badge: preset.factory ? t('presets.factory') : null,
            }))}
            value={chosen}
            onChange={(id) => {
              setChosen(id);
              load(id);
            }}
            placeholder={t('presets.name')}
          />

          {/* Absent rather than disabled for factory presets: a control that is
              there and always refuses is a worse answer than no control. */}
          {chosenPreset !== undefined && !chosenPreset.factory && (
            <button
              type="button"
              className="presets__delete"
              aria-label={t('presets.delete', { name: chosenPreset.name })}
              title={t('presets.delete', { name: chosenPreset.name })}
              onClick={() => remove(chosenPreset.id)}
            >
              <Trash2 size={13} aria-hidden="true" />
            </button>
          )}
        </div>
      )}

      {presets.length === 0 && !error && <p className="presets__empty">{t('presets.none')}</p>}

      <div className="presets__save">
        <input
          type="text"
          className="presets__input"
          value={name}
          maxLength={64}
          placeholder={t('presets.name')}
          aria-label={t('presets.name')}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') save();
          }}
        />
        <button
          type="button"
          className="presets__button"
          disabled={name.trim() === ''}
          onClick={() => save()}
        >
          {t('presets.save')}
        </button>
      </div>

      {error !== null && <p className="presets__error">{error}</p>}
    </div>
  );
}
