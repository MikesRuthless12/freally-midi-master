import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { CornerLeftUp, FolderOpen, FolderPlus, Music, X } from 'lucide-react';

import { useExplorer } from '../../state/explorer';
import { PreviewPlayer } from './PreviewPlayer';
import './Explorer.css';

/**
 * The sample browser (TASK-132).
 *
 * ⛔ **A library of saved folders, not a folder picker.** Mike chose that on
 * 2026-08-07: a library is set up once, not once per project, and re-picking it
 * every session is what makes a browser not worth opening. The roots persist
 * with the project; the position inside them deliberately does not.
 *
 * ⛔ **The rows are a drag source**, which is the whole point of building this
 * before the roster work. Mike, 2026-08-06: *"when we do the 'File Explorer'
 * then we will be able to drop samples on the generators and drum lanes."*
 * `KitPanel` is the drop target and `explorer_drop` is the landing.
 *
 * ⚠ **The drag is the WebView's own HTML5 drag, not the native `DoDragDrop`
 * one.** They are different mechanisms in both directions: dragging a clip *out*
 * to a DAW needs `CF_HDROP` and a real OLE source (`plugin/src/drag.rs`);
 * dragging a row onto a lane inside the same page needs neither and must not
 * reach for it.
 */
export function ExplorerPanel() {
  const { t } = useTranslation();
  const roots = useExplorer((s) => s.roots);
  const folder = useExplorer((s) => s.folder);
  const parent = useExplorer((s) => s.parent);
  const entries = useExplorer((s) => s.entries);
  const truncated = useExplorer((s) => s.truncated);
  const loaded = useExplorer((s) => s.loaded);
  const selected = useExplorer((s) => s.selected);
  const error = useExplorer((s) => s.error);
  const refresh = useExplorer((s) => s.refresh);
  const addFolder = useExplorer((s) => s.addFolder);
  const removeFolder = useExplorer((s) => s.removeFolder);
  const open = useExplorer((s) => s.open);
  const select = useExplorer((s) => s.select);
  const play = useExplorer((s) => s.play);
  const setReverse = useExplorer((s) => s.setReverse);

  // Read once when the panel mounts. `Section` unmounts a collapsed panel's
  // content, so reopening it re-reads — which is what keeps the list in step
  // with a folder added while it was shut.
  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <div
      className="browser"
      // ⛔ **Scoped to this panel, deliberately, and not bound on `window`.**
      // Mike asked for *"left arrow plays the sample backwards when the file is
      // selected"* — but `PianoRoll/shortcuts.ts` already binds ← and → to nudge
      // the selected notes, so a global listener would take that away from the
      // editor. Clicking a row leaves focus on that row's button, which is
      // inside this container, so "when the file is selected" is exactly when
      // this fires and never when the producer is editing notes.
      onKeyDown={(event) => {
        if (selected === null) return;
        if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
        if (event.ctrlKey || event.metaKey || event.altKey) return;
        // Otherwise the arrow also walks the focus ring along the row buttons
        // while the sample plays, which moves the selection out from under it.
        event.preventDefault();
        const backwards = event.key === 'ArrowLeft';
        void setReverse(backwards).then(() => play());
      }}
    >
      <div className="browser__roots">
        {roots.map((root) => (
          <span
            key={root.path}
            className="browser__root"
            data-current={folder !== null && folder.startsWith(root.path)}
          >
            <button
              type="button"
              className="browser__root-open"
              title={root.path}
              onClick={() => void open(root.path)}
            >
              <FolderOpen size={12} aria-hidden="true" />
              {root.name}
            </button>
            <button
              type="button"
              className="browser__root-remove"
              aria-label={t('explorer.removeFolder', { name: root.name })}
              onClick={() => void removeFolder(root.path)}
            >
              <X size={11} aria-hidden="true" />
            </button>
          </span>
        ))}
        <button type="button" className="browser__add" onClick={() => void addFolder()}>
          <FolderPlus size={12} aria-hidden="true" />
          {t('explorer.addFolder')}
        </button>
      </div>

      {roots.length === 0 && loaded && <p className="browser__hint">{t('explorer.noRoots')}</p>}

      {folder !== null && (
        <div className="browser__crumb">
          {/* ⚠ Absent at a root, and that is the containment boundary showing
              through rather than a missing button: `Explorer::state` nulls the
              parent there so "up" cannot walk out of the library into somebody's
              home directory. */}
          <button
            type="button"
            className="btn-ghost browser__up"
            disabled={parent === null}
            onClick={() => parent !== null && void open(parent)}
          >
            <CornerLeftUp size={12} aria-hidden="true" />
            {t('explorer.up')}
          </button>
          {/* The folder's own name, because the rail is narrow and the full path
              is what the tooltip is for. */}
          <span className="browser__here" title={folder}>
            {folder.split(/[\\/]/).filter(Boolean).pop() ?? folder}
          </span>
        </div>
      )}

      {folder !== null && (
        <ul className="browser__list" aria-label={t('explorer.listLabel')}>
          {entries.map((entry) => (
            <li key={entry.path} className="browser__row">
              <button
                type="button"
                className="browser__entry"
                data-kind={entry.isDir ? 'dir' : 'file'}
                data-selected={selected === entry.path}
                title={entry.name}
                // ⛔ Files are draggable, folders are not — there is nothing a
                // folder could mean when dropped on one drum lane.
                draggable={!entry.isDir}
                onDragStart={(event) => {
                  // ⛔ **`text/plain` as well as the private type.** Some
                  // WebView2 builds refuse to start a drag whose DataTransfer
                  // carries only an unrecognised MIME type, and the drag then
                  // silently never begins.
                  event.dataTransfer.setData('application/x-freally-sample', entry.path);
                  event.dataTransfer.setData('text/plain', entry.path);
                  event.dataTransfer.effectAllowed = 'copy';
                }}
                onClick={() => void (entry.isDir ? open(entry.path) : select(entry.path))}
              >
                {entry.isDir ? (
                  <FolderOpen size={12} aria-hidden="true" />
                ) : (
                  <Music size={12} aria-hidden="true" />
                )}
                <span className="browser__name">{entry.name}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {folder !== null && entries.length === 0 && (
        <p className="browser__hint">{t('explorer.empty')}</p>
      )}

      {/* ⚠ **Reported, never a silent cut.** A list that stops at 2000 with
          nothing saying so is the failure `MAX_ENTRIES` documents: the producer
          scrolls, does not find their kick, and concludes the browser is
          broken. */}
      {truncated && <p className="browser__hint">{t('explorer.truncated')}</p>}

      <PreviewPlayer />

      {error && (
        <p className="kit-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
