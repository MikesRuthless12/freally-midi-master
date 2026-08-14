import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CornerLeftUp, FolderPlus, RotateCw, Search, X } from 'lucide-react';

import {
  MAX_ROOTS,
  flattenTree,
  innermostExpanded,
  isInside,
  samePath,
  useExplorer,
} from '../../state/explorer';
import { useKit } from '../../state/kit';
import { useSession } from '../../state/session';
import { padsOf, useUi } from '../../state/ui';
import { Favourites } from './Favourites';
import { Recent } from './Recent';
import { FileTree } from './FileTree';
import { PreviewPlayer } from './PreviewPlayer';
import type { Lane } from '../../lib/ipc-types';
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
  const loadRecent = useExplorer((s) => s.loadRecent);
  const addFolder = useExplorer((s) => s.addFolder);
  const removeFolder = useExplorer((s) => s.removeFolder);
  const collapse = useExplorer((s) => s.collapse);
  const activeRoot = useExplorer((s) => s.activeRoot);
  const setActiveRoot = useExplorer((s) => s.setActiveRoot);
  const toggleFolder = useExplorer((s) => s.toggleFolder);
  const select = useExplorer((s) => s.select);
  const play = useExplorer((s) => s.play);
  const setReverse = useExplorer((s) => s.setReverse);
  const dropOn = useExplorer((s) => s.dropOn);
  const children = useExplorer((s) => s.children);
  const truncatedIn = useExplorer((s) => s.truncatedIn);
  const missingRoots = useExplorer((s) => s.missingRoots);

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
    void loadRecent();
  }, [refresh, loadFavourites, loadRecent]);

  /**
   * The lane `Ctrl`+arrow will put the selected sample on, or `null`.
   *
   * ⛔⛔ **Two different answers, and Mike drew the line himself** (2026-08-11):
   * *"you should be able to select one of the drum pads, and then 'Ctrl + left
   * arrow' … that same goes with the melody/chords/bassline/counter melody, but
   * **you should only have to be in their tab** to be able to add them to the
   * melodic parts, you should not have to select the actual 'Melody' button
   * where you drag it to."*
   *
   * So on the **Drums** tab the target is whichever pad is selected — which is
   * why that selection is never allowed to be empty — and on a melodic tab it is
   * that tab's own lane, with nothing to aim.
   *
   * ⚠ **`null` on the Song tab**, and that is the honest answer rather than a
   * fallback: an arrangement has no one lane to drop a one-shot onto, and
   * quietly picking the drums would put a sample somewhere the producer was not
   * looking.
   */
  const activeTab = useUi((s) => s.activeTab);
  const selectedPad = useUi((s) => s.selectedPad);
  const padsByStyle = useUi((s) => s.pads);
  const selectedId = useSession((s) => s.selectedId);
  const refreshKit = useKit((s) => s.refresh);
  const selectedKind = useExplorer((s) => s.selectedKind);
  const assignTarget: Lane | null =
    activeTab === 'song'
      ? null
      : activeTab === 'drums'
        ? ((padsOf(padsByStyle, selectedId)[selectedPad] ?? null) as Lane | null)
        : (activeTab as Lane);

  /** Shut the deepest open folder. What `Up` and `←`-on-a-folder both do. */
  const up = () => {
    const deepest = innermostExpanded(expanded);
    if (deepest !== null) collapse(deepest);
  };

  // ⚠ **Falls back to the first tab rather than showing nothing.** `activeRoot`
  // starts `null` and a removed root leaves it naming a folder that is gone; in
  // both cases the honest answer is the first library folder there is, not an
  // empty panel that looks like a browser with no library.
  const shown = roots.find((root) => samePath(activeRoot, root.path)) ?? roots[0] ?? null;
  const isMissing = shown !== null && missingRoots.some((held) => samePath(held, shown.path));

  /**
   * What the producer has typed into the filter box (TASK-058).
   *
   * ⚠ **Component state, like `focused` below and for the same reason.** It sat
   * in the explorer store briefly, on the grounds that `FileTree` needed it too —
   * which is not true: the tree takes `rows` as a prop and `flattenTree` takes
   * the query as a parameter, so this has exactly one reader. In the store it
   * would also outlive the panel, and `Section` unmounts a collapsed one — so
   * closing and reopening the browser would bring back a filter typed a while
   * ago, with the tree narrowed and nothing on screen saying why.
   */
  const [filter, setFilter] = useState('');

  /**
   * Every line the tree draws, flattened (TASK-058).
   *
   * ⛔⛔ **Computed once, here, and handed to both readers.** `FileTree` draws a
   * window of it and the ↑/↓ walk below steps through it. That walk used to read
   * `.tree__row` out of the DOM in document order, which was correct only while
   * every row was mounted — under virtualization the DOM holds ~30 rows of
   * however many thousand, so the walk would have stopped dead at the edge of
   * the window.
   *
   * ⚠ Memoized on the slices it reads, because `subscribeToPreview` writes
   * `position` at 30 Hz while a sample auditions and this panel subscribes to it.
   */
  const shownPath = shown?.path ?? null;
  const shownName = shown?.name ?? '';
  const rows = useMemo(
    () =>
      shownPath === null
        ? []
        : flattenTree(
            { name: shownName, path: shownPath, isDir: true, kind: 'dir' },
            { expanded, children, truncatedIn, query: filter },
          ),
    // ⛔⛔ **Keyed on the root's PATH, not on the root object.** `refresh` sets
    // `roots` to the reply's array, and the folder-dialog poll calls it every
    // 400 ms — so `shown` is a new object two and a half times a second and a
    // memo that depended on it would re-walk every expanded folder, thousands of
    // rows at a time, for the whole minute somebody spends in a file picker.
    // The path and the name are all `flattenTree` reads of the root.
    [shownPath, shownName, expanded, children, truncatedIn, filter],
  );

  /**
   * The row the keyboard is on, by path, or `null`.
   *
   * ⛔ **Not the same as the selection, and it never was.** A folder is stepped
   * *over* by ↑/↓ rather than selected — it has no waveform and nothing to
   * audition — so the highlight and the focus ring are two positions, and
   * collapsing them would clear the sample a producer is walking past a folder
   * to get back to.
   */
  const [focused, setFocused] = useState<string | null>(null);

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

        // ⛔⛔ **`Ctrl`+←/→ PUTS THE SELECTED SAMPLE ON THE CURRENT TARGET** —
        // Mike, 2026-08-11: *"you should be able to select one of the drum pads,
        // and then 'Ctrl + left arrow' when you have a sample selected should add
        // the sample to that selected drum pad lane **in reverse** and 'Ctrl +
        // right arrow' should add the sample to that selected drum pad lane
        // playing regularly, that same goes with the melody/chords/bassline/
        // counter melody, but you should only have to be in their tab."*
        //
        // ⚠ **The plain arrows already mean two other things here** — ↑/↓ walk
        // the tree and ←/→ audition backwards and forwards — and this handler
        // used to `return` on any modifier, which is exactly what left `Ctrl`
        // free to mean something new.
        if (event.ctrlKey || event.metaKey) {
          const sideways = event.key === 'ArrowLeft' || event.key === 'ArrowRight';
          // ⚠ Nothing selected, or a `.mid`, or the Song tab — all three are
          // "there is no target", and none of them is worth a message: the
          // producer pressed a shortcut in a state it does not apply to.
          if (
            !sideways ||
            assignTarget === null ||
            selected === null ||
            selectedKind !== 'audio'
          ) {
            return;
          }
          event.preventDefault();
          void dropOn(assignTarget, selected, event.key === 'ArrowLeft').then(() =>
            refreshKit(),
          );
          return;
        }

        if (event.altKey) return;

        // ⚠ **The focused path comes from state, not from the DOM.** Under
        // virtualization the row a producer walked to may already have been
        // unmounted by a scroll, and `closest('.tree__row')` would then answer
        // `null` for a row that is very much still where the keyboard is.
        const inTree =
          event.target instanceof HTMLElement && event.target.closest('.tree__row') !== null;
        const path = inTree ? focused : null;
        const at = path === null ? -1 : rows.findIndex((held) => held.entry?.path === path);
        const entry = at >= 0 ? (rows[at]?.entry ?? null) : null;
        const backwards = event.key === 'ArrowLeft';

        // ⛔ **↑/↓ walk the tree** (TASK-058A): *"`↑`/`↓` move the selection, so a
        // producer can walk a folder and hear every file — in both directions —
        // without touching the mouse."*
        //
        // ⛔⛔ **IT SELECTS AS IT WALKS NOW, AND THE SAMPLE PLAYS** (Mike,
        // 2026-08-11): *"the files need to play as you go up and down in the list
        // with the up/down arrow or by clicking on them."*
        //
        // ⚠ **This reverses the note that stood here**, which read: *"Moves focus
        // and does not audition. Pairing it with → is what makes walking a folder
        // possible at all … auditioning on every step would make ↓ unusable for
        // simply getting past a folder."* The objection was reasonable and Mike
        // has overruled it by name. ▶ It also left ↓ doing **less** than it
        // looked like it did: the row took focus and the selection did not move
        // at all, so the waveform and the transport went on describing whatever
        // was last clicked while the highlight walked away from it.
        //
        // ⚠ **The playing is `select`'s**, not this handler's — a click has to do
        // the same thing and there must not be two answers to when a file
        // sounds. → still auditions, for anyone who has the old gesture in their
        // fingers.
        //
        // ⛔⛔ **Over the FLATTENED rows, which are what is drawn.** This used to
        // walk `.tree__row` in document order — correct only while the whole
        // tree was mounted. `flattenTree` produces the lines in exactly the
        // tree's visual order, so this is the same walk over the model the
        // window is cut from rather than over the cut.
        //
        // ⚠ **Entries only.** "Reading…", "no samples" and the truncation notice
        // are rows on screen but there is nothing to land on: focusing one is a
        // dead stop mid-walk, and auditioning one is meaningless.
        if (vertical) {
          event.preventDefault();
          const step = event.key === 'ArrowDown' ? 1 : -1;
          // ⚠ **Stepped over `rows` in place**, rather than filtering it into a
          // second array first: at the plugin's own bound that allocation is
          // 2,000 entries per keypress, and holding ↓ makes it per key-repeat.
          // Clamped rather than wrapped — a list that jumps from the last row
          // back to the first reads as the key having done nothing.
          let next = at < 0 ? (step > 0 ? 0 : rows.length - 1) : at + step;
          while (next >= 0 && next < rows.length && rows[next]?.entry == null) next += step;
          const landed = next >= 0 && next < rows.length ? rows[next]?.entry : null;
          // Off either end, or a tree of nothing but status lines: stay put.
          if (landed === undefined || landed === null) return;
          // ⚠ **Focus is set here and applied by `FileTree`**, which has to draw
          // the row before anything can focus it — the row a walk lands on may be
          // outside the current window.
          setFocused(landed.path);
          // ⚠ **A folder is stepped over, not selected.** It has no waveform and
          // nothing to audition, and selecting one would clear the sample the
          // producer is walking past it to get back to.
          if (!landed.isDir) void select(landed.path);
          return;
        }

        // ⛔⛔ **The same two keys mean different things on a folder and on a
        // file, and they do not collide** — Mike, 2026-08-10: *"pressing the
        // right arrow should expand a folder, and pressing right arrow on a
        // sample/one shot/midi should play the midi and pressing the left arrow
        // should play the midi/sample/one shot backwards."* A folder has nothing
        // to audition and a sample has nothing to expand, so which one the key
        // means is never ambiguous — it is decided by the row under the focus.
        if (entry?.isDir === true && path !== null) {
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
        if (target === null) return;
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

        {/* ⚠ **The "fill the rail" toggle is gone, and the feature it asked for
            is now unconditional** (2026-08-11). It existed because the browser
            and the roster were both `Section … grow` and each got half the rail
            however deep the folder tree ran — Mike, 2026-08-10: *"the File
            Explorer needs to show and hide the 'Artist/Producer Roster' part so
            that way it can expand as high as it can."* Under `RAIL_GROUPS` the
            browser is a group of **one**, so showing it *is* giving it the whole
            rail. A button that collapsed the roster would now be a second way to
            do what its own tab already does. */}
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
              // ⚠ **The tab keeps its place when the drive is unplugged.**
              // `explorer::merge_folders` keeps such a root deliberately — a
              // producer who unplugged their sample disk has not left the
              // library — so this says so rather than removing it.
              data-missing={missingRoots.some((held) => samePath(held, root.path))}
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

      {/* ⛔⛔ **Type-to-filter** (TASK-058), and it says what it searched.
          `children` only holds folders the producer has expanded — the plugin
          reads one folder per call, because walking a whole library on the
          host's editor thread is the thing `Explorer::list_one` refuses to do —
          so this narrows *what has been read*. A box that looked like it
          searched the entire library while searching part of it is the
          readout-that-lies failure, which is why the scope line below is not
          optional. */}
      {roots.length > 0 && !isMissing && (
        <div className="browser__filter">
          <Search size={12} aria-hidden="true" />
          <input
            type="search"
            value={filter}
            placeholder={t('explorer.filter')}
            aria-label={t('explorer.filter')}
            onChange={(event) => setFilter(event.target.value)}
            // ⚠ Escape clears it, which is the gesture every filter box has —
            // and without it the only way back to the whole tree is to select
            // the text and delete it.
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.stopPropagation();
                setFilter('');
              }
            }}
          />
        </div>
      )}
      {/* ⚠ **Two lines, because they answer two questions.** The scope says
          what the filter can reach; the second says this particular query
          reached nothing. `rows.length <= 1` is the no-match case by
          construction — `flattenTree` keeps the root row so the panel does not
          go blank, and nothing else survives. */}
      {filter.trim() !== '' && !isMissing && (
        <p className="browser__scope">
          {t('explorer.filterScope')}
          {rows.length <= 1 && ` ${t('explorer.noMatches')}`}
        </p>
      )}

      {/* ⛔ **A folder that is not there says so, rather than refusing to
          open.** Before this it drew as an ordinary root, expanding it failed
          with the one refusal message every failure shares, and the twisty shut
          again — so an unplugged drive was indistinguishable from an empty
          folder. `Check again` is a re-read rather than anything cleverer: the
          fix for an unplugged drive is to plug it back in, and this is how the
          panel notices. */}
      {isMissing && shown !== null && (
        <p className="browser__missing" role="status">
          {t('explorer.missing', { name: shown.name })}
          <button type="button" className="btn-ghost" onClick={() => void refresh()}>
            <RotateCw size={11} aria-hidden="true" />
            {t('explorer.recheck')}
          </button>
        </p>
      )}

      {roots.length > 0 && !isMissing && (
        <FileTree rows={rows} focused={focused} onFocusRow={setFocused} />
      )}

      {/* ⚠ **Between the tree and the transport**, so it is the thing a producer
          drops to when they know what they are after — and above the player, so
          clicking a favourite and hearing it are next to each other. */}
      <Favourites />
      <Recent />

      <PreviewPlayer />

      {error && (
        <p className="kit-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
