import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { CornerLeftUp, FolderPlus, Maximize2, Minimize2, X } from 'lucide-react';

import {
  MAX_ROOTS,
  innermostExpanded,
  isInside,
  samePath,
  useExplorer,
} from '../../state/explorer';
import { useUi } from '../../state/ui';
import { Favourites } from './Favourites';
import { FileTree } from './FileTree';
import { PreviewPlayer } from './PreviewPlayer';
import './Explorer.css';

/**
 * The sample browser (TASK-132).
 *
 * ⛔ **A library of saved folders, not a folder picker.** Mike chose that on
 * 2026-08-07: a library is set up once, not once per project, and re-picking it
 * every session is what makes a browser not worth opening. The roots persist
 * with the project; which of them are *expanded* deliberately does not.
 *
 * ⛔ **The rows are a drag source**, which is the whole point of building this
 * before the roster work. Mike, 2026-08-06: *"when we do the 'File Explorer'
 * then we will be able to drop samples on the generators and drum lanes."*
 * `PadGrid` is the drop target and `explorer_drop` is the landing.
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
  const loaded = useExplorer((s) => s.loaded);
  const selected = useExplorer((s) => s.selected);
  const expanded = useExplorer((s) => s.expanded);
  const error = useExplorer((s) => s.error);
  const refresh = useExplorer((s) => s.refresh);
  const loadFavourites = useExplorer((s) => s.loadFavourites);
  const addFolder = useExplorer((s) => s.addFolder);
  const removeFolder = useExplorer((s) => s.removeFolder);
  const collapse = useExplorer((s) => s.collapse);
  const activeRoot = useExplorer((s) => s.activeRoot);
  const setActiveRoot = useExplorer((s) => s.setActiveRoot);
  const toggleFolder = useExplorer((s) => s.toggleFolder);
  const select = useExplorer((s) => s.select);
  const play = useExplorer((s) => s.play);
  const setReverse = useExplorer((s) => s.setReverse);

  // ⛔⛔ **The browser can take the whole rail** — Mike, 2026-08-10: *"the File
  // Explorer needs to show and hide the 'Artist/Producer Roster' part so that
  // way it can expand as high as it can, not just a little bit or half-way up
  // the side of the left rail."*
  //
  // ▶ **Why it was only ever half.** The roster and the browser are both
  // `Section … grow`, and `.rail__section--grow` is `flex: 1 1 0` — two equal
  // claims on the rail, so each got half of it however deep the folder tree ran.
  // Collapsing the roster by its own header already freed that space; what did
  // not exist was any way to know that from the browser, which is where a
  // producer is standing when they run out of room.
  //
  // ⚠ **It drives the sections that already exist rather than adding a mode.**
  // `Section` unmounts collapsed content and the state is the one `K` and the
  // View menu already write, so this is the same show/hide by another button —
  // not a second layout the two could disagree about.
  const sections = useUi((s) => s.sections);
  const toggleSection = useUi((s) => s.toggleSection);
  const crowded = sections.roster || sections.genres;

  // Read once when the panel mounts. `Section` unmounts a collapsed panel's
  // content, so reopening it re-reads — which is what keeps the list in step
  // with a folder added while it was shut.
  //
  // ⚠ **The stars are read here and nowhere else.** They used to be fetched
  // inside `refresh`, which runs on the folder-dialog poll every 400 ms and on
  // every twisty click — a file read and a JSON parse on the host's editor
  // thread, for a list that only changes when somebody presses a star. Starring
  // hands the fresh list straight back, so this is the only other moment it can
  // have changed under us.
  useEffect(() => {
    void refresh();
    void loadFavourites();
  }, [refresh, loadFavourites]);

  /** Shut the deepest open folder. What `Up` and `←`-on-a-folder both do. */
  const up = () => {
    const deepest = innermostExpanded(expanded);
    if (deepest !== null) collapse(deepest);
  };

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
        const vertical = event.key === 'ArrowUp' || event.key === 'ArrowDown';
        if (!vertical && event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
        if (event.ctrlKey || event.metaKey || event.altKey) return;

        const row =
          event.target instanceof HTMLElement
            ? event.target.closest<HTMLElement>('.tree__row')
            : null;
        const path = row?.dataset.path;
        const backwards = event.key === 'ArrowLeft';

        // ⛔ **↑/↓ walk the tree** (TASK-058A): *"`↑`/`↓` move the selection, so a
        // producer can walk a folder and hear every file — in both directions —
        // without touching the mouse."*
        //
        // ⚠ **Moves focus and does not audition.** Pairing it with → is what
        // makes walking a folder possible at all: a producer steps down the rows
        // and presses → on the ones worth hearing. Auditioning on every step is
        // TASK-058's *"arrow down a folder and every file plays as you land on
        // it"* — a different behaviour, and one that would make ↓ unusable for
        // simply getting past a folder.
        //
        // ⚠ **Over the rendered rows, in document order**, which is exactly the
        // tree's visual order — a flattened model would be a second idea of what
        // is on screen, and the one that drifts.
        if (vertical) {
          const rows = Array.from(
            event.currentTarget.querySelectorAll<HTMLElement>('.tree__row'),
          );
          if (rows.length === 0) return;
          event.preventDefault();
          const at = row === null ? -1 : rows.indexOf(row);
          const step = event.key === 'ArrowDown' ? 1 : -1;
          // Clamped rather than wrapped: a list that jumps from the last row back
          // to the first reads as the key having done nothing.
          const next = Math.min(rows.length - 1, Math.max(0, at + step));
          rows[next]?.focus();
          return;
        }

        // ⛔⛔ **The same two keys mean different things on a folder and on a
        // file, and they do not collide** — Mike, 2026-08-10: *"pressing the
        // right arrow should expand a folder, and pressing right arrow on a
        // sample/one shot/midi should play the midi and pressing the left arrow
        // should play the midi/sample/one shot backwards."* A folder has nothing
        // to audition and a sample has nothing to expand, so which one the key
        // means is never ambiguous — it is decided by the row under the focus.
        if (row?.dataset.kind === 'dir' && path !== undefined) {
          event.preventDefault();
          const isOpen = expanded.some((held) => samePath(held, path));
          if (!backwards) {
            // → opens it. Already open is not a no-op worth guarding — the
            // toggle would *shut* it, which is the opposite of what was asked.
            if (!isOpen) void toggleFolder(path);
            return;
          }
          // ← shuts it; on one that is already shut, it shuts the branch this
          // folder is *in*, which is how every tree behaves and is what makes ←
          // a way back out rather than a key that sometimes does nothing.
          if (isOpen) {
            collapse(path);
            return;
          }
          const parent = innermostExpanded(expanded.filter((held) => isInside(path, held)));
          if (parent !== null) collapse(parent);
          return;
        }

        // ⚠ **The focused row wins over the selected one.** Walking the tree
        // with the keyboard moves focus without selecting, so reading `selected`
        // alone would audition the file the producer had already left.
        const target = path ?? selected;
        if (target === undefined || target === null) return;
        // Otherwise the arrow also walks the focus ring along the row buttons
        // while the sample plays, which moves the selection out from under it.
        event.preventDefault();
        const start = () => setReverse(backwards).then(() => play());
        // ⛔ Loaded first when it is not the one already loaded, or the transport
        // would sound the previous sample under the new row's name.
        if (target !== selected) void select(target).then(start);
        else void start();
      }}
    >
      <div className="browser__bar">
        {/* ⛔⛔ **Disabled at eight rather than refused on press** — Mike,
            2026-08-10: *"the add folder button needs to be disabled if you have
            8 folders until you remove one folder."* A button that opens a dialog
            and then rejects what you picked is the worse half of both; a
            disabled one states the rule before you spend the gesture.
            ⚠ The plugin refuses a ninth independently — this is a UI state and
            `explorer::MAX_ROOTS` is the real bound. */}
        <button
          type="button"
          className="browser__add"
          disabled={roots.length >= MAX_ROOTS}
          title={roots.length >= MAX_ROOTS ? t('explorer.folderLimit') : undefined}
          onClick={() => void addFolder()}
        >
          <FolderPlus size={12} aria-hidden="true" />
          {t('explorer.addFolder')}
        </button>

        {/* ⚠ Disabled rather than hidden when nothing is expanded: a control
            that comes and goes is one a producer has to look for twice. */}
        <button
          type="button"
          className="btn-ghost browser__up"
          disabled={innermostExpanded(expanded) === null}
          onClick={up}
        >
          <CornerLeftUp size={12} aria-hidden="true" />
          {t('explorer.up')}
        </button>

        <button
          type="button"
          className="btn-ghost browser__grow"
          aria-pressed={!crowded}
          title={t(crowded ? 'explorer.fillRail' : 'explorer.restoreRail')}
          aria-label={t(crowded ? 'explorer.fillRail' : 'explorer.restoreRail')}
          onClick={() => {
            // ⚠ Both of them, and back again. The genres combobox is only one
            // row, but with the roster shut it is the only thing left between
            // the browser and the top of the rail.
            if (sections.roster !== !crowded) toggleSection('roster');
            if (sections.genres !== !crowded) toggleSection('genres');
          }}
        >
          {crowded ? (
            <Maximize2 size={12} aria-hidden="true" />
          ) : (
            <Minimize2 size={12} aria-hidden="true" />
          )}
        </button>
      </div>

      {/* ⛔⛔ **The library folders are tabs, up to eight** — Mike, 2026-08-10:
          *"i should be able to have up to 8 folders in the view to be able to be
          tabbed to be used and sifted through at any given time."* They were
          stacked as top-level rows of one tree, which meant eight libraries put
          eight rows between the producer and whatever they were actually looking
          at — and cost the tree the vertical room the same producer had just
          asked to get back.
          ⚠ **The close button is on the tab**, so a click on the tab itself is
          always "sift this one" and never "lose it". */}
      {roots.length > 0 && (
        <div className="browser__tabs" role="tablist" aria-label={t('sections.explorer')}>
          {roots.map((root) => (
            <span
              key={root.path}
              className="browser__tab"
              data-current={samePath(activeRoot ?? roots[0]?.path ?? null, root.path)}
            >
              <button
                type="button"
                role="tab"
                aria-selected={samePath(activeRoot ?? roots[0]?.path ?? null, root.path)}
                className="browser__tab-open"
                title={root.path}
                onClick={() => setActiveRoot(root.path)}
              >
                {root.name}
              </button>
              <button
                type="button"
                className="browser__tab-close"
                aria-label={t('explorer.removeFolder', { name: root.name })}
                title={t('explorer.removeFolder', { name: root.name })}
                onClick={() => void removeFolder(root.path)}
              >
                <X size={10} aria-hidden="true" />
              </button>
            </span>
          ))}
        </div>
      )}

      {roots.length === 0 && loaded && <p className="browser__hint">{t('explorer.noRoots')}</p>}

      {roots.length > 0 && <FileTree />}

      {/* ⚠ **Between the tree and the transport**, so it is the thing a producer
          drops to when they know what they are after — and above the player, so
          clicking a favourite and hearing it are next to each other. */}
      <Favourites />

      <PreviewPlayer />

      {error && (
        <p className="kit-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
