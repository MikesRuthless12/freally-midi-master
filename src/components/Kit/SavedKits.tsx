import { Copy, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { invoke } from '../../lib/ipc';
import { useKit } from '../../state/kit';

/**
 * Named kits (TASK-051).
 *
 * A producer's one-shots already persist **with the project** — reopen a song
 * and the samples come back. This is the other half: the same assignments,
 * named, kept outside any project, so a kit built on one track can be put on the
 * next one. Exactly the distinction [`Presets`] draws between "this song" and
 * "any song", which is why this panel is shaped like that one.
 *
 * ⛔ **It saves paths, not audio.** A saved kit is a few hundred bytes naming
 * files that already exist on the producer's disk. Copying the samples is a
 * separate, *consented* thing that belongs to a style — doing it silently on a
 * Save button would spend somebody's disk space without asking, which is the
 * thing the owner explicitly ruled out.
 */
type KitSummary = {
  id: string;
  name: string;
  lanes: number;
};

export function SavedKits() {
  const { t } = useTranslation();
  const awaitLoader = useKit((s) => s.awaitLoader);

  const [kits, setKits] = useState<KitSummary[]>([]);
  const [name, setName] = useState('');
  const [error, setError] = useState<string | null>(null);

  const fail = (cause: unknown) =>
    setError(cause instanceof Error ? cause.message : String(cause));

  const refresh = () => {
    invoke<KitSummary[]>('kits_list')
      .then((list) => {
        setKits(list);
        setError(null);
      })
      .catch(fail);
  };

  useEffect(refresh, []);

  const save = () => {
    const trimmed = name.trim();
    if (trimmed === '') return;
    invoke('kits_save', { name: trimmed })
      .then(() => {
        setName('');
        refresh();
      })
      .catch(fail);
  };

  const load = (id: string) => {
    // ⛔ **Waits for the loader thread rather than refreshing straight away.**
    // `kits_load` hands the decode off and answers immediately, so a resolved
    // promise means "started". Refreshing here read the panel mid-decode, and a
    // kit whose samples had moved published its failure into a slot nothing
    // read — no error on screen and unchanged pads.
    invoke('kits_load', { id })
      .then(() => awaitLoader())
      .catch(fail);
  };

  return (
    <div className="presets">
      <ul className="presets__list">
        {kits.map((kit) => (
          <li key={kit.id} className="presets__item">
            <button type="button" className="presets__load" onClick={() => load(kit.id)}>
              <span className="presets__name">{kit.name}</span>
              {/* How many lanes it carries, so two kits are tellable apart at a
                  glance rather than only by name. */}
              <span className="presets__badge">{kit.lanes}</span>
            </button>

            {/* ⛔ **Duplicate and rename both use the name box**, because a
                plugin webview has no prompt worth showing and a second inline
                editor for one field would be more UI than the job needs. With
                the box empty they are absent rather than present-and-failing —
                the same rule the factory presets follow one panel up. */}
            {name.trim() !== '' && (
              <button
                type="button"
                className="presets__delete"
                aria-label={t('kits.duplicate', { name: kit.name })}
                title={t('kits.duplicate', { name: kit.name })}
                onClick={() =>
                  invoke('kits_duplicate', { id: kit.id, name: name.trim() })
                    .then(() => {
                      setName('');
                      refresh();
                    })
                    .catch(fail)
                }
              >
                <Copy size={13} aria-hidden="true" />
              </button>
            )}

            <button
              type="button"
              className="presets__delete"
              aria-label={t('kits.delete', { name: kit.name })}
              onClick={() => invoke('kits_delete', { id: kit.id }).then(refresh).catch(fail)}
            >
              <Trash2 size={13} aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>

      {kits.length === 0 && error === null && (
        <p className="presets__empty">{t('kits.none')}</p>
      )}

      <div className="presets__save">
        <input
          type="text"
          className="presets__input"
          value={name}
          maxLength={64}
          placeholder={t('kits.name')}
          aria-label={t('kits.name')}
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
          {t('kits.save')}
        </button>
      </div>

      {error !== null && <p className="presets__error">{error}</p>}
    </div>
  );
}
