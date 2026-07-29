import { Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import { useSession, type SavedSession } from '../../state/session';
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
      <ul className="presets__list">
        {presets.map((preset) => (
          <li key={preset.id} className="presets__item">
            <button type="button" className="presets__load" onClick={() => load(preset.id)}>
              <span className="presets__name">{preset.name}</span>
              {preset.factory && <span className="presets__badge">{t('presets.factory')}</span>}
            </button>

            {/* Absent rather than disabled for factory presets: a control that
                is there and always refuses is a worse answer than no control. */}
            {!preset.factory && (
              <button
                type="button"
                className="presets__delete"
                aria-label={t('presets.delete', { name: preset.name })}
                onClick={() => remove(preset.id)}
              >
                <Trash2 size={13} aria-hidden="true" />
              </button>
            )}
          </li>
        ))}
      </ul>

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
