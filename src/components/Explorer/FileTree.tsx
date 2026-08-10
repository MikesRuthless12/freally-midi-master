import {
  ChevronDown,
  ChevronRight,
  FileMusic,
  Folder,
  FolderOpen,
  Music,
  Star,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { MIDI_TYPE, SAMPLE_TYPE } from '../../lib/dnd';
import { samePath, useExplorer, type ExplorerEntry } from '../../state/explorer';

/**
 * How far one level indents, in pixels.
 *
 * ⚠ Small on purpose. The rail starts at 280px and a sample library is easily
 * six levels deep — at the 16px a desktop file manager uses, the sixth level
 * would have under half the rail left to write a filename in.
 */
const INDENT = 10;

/**
 * The tree (TASK-058).
 *
 * ⛔⛔ **This replaces a one-folder-at-a-time list, on Mike's instruction**,
 * 2026-08-10: *"you need to show it like a real file explorer does, where the
 * main folder is at the top, and then indents and shows the subfolders
 * underneath and then the files beneath that."*
 *
 * The list it replaced showed one folder and swapped its whole contents when you
 * clicked into a subfolder, so the only way to know where you were was the
 * breadcrumb — and the only way back was `Up`. That is a navigator, and it is
 * the shape Mike's own complaint was about.
 *
 * ⛔ **Folders sort above files at every level**, which is `explorer::list`'s
 * doing rather than this component's — one ordering, in the place that reads the
 * directory, so the tree cannot disagree with what the plugin thinks it sent.
 */
export function FileTree() {
  const { t } = useTranslation();
  const roots = useExplorer((s) => s.roots);
  const activeRoot = useExplorer((s) => s.activeRoot);

  // ⛔⛔ **Every slice a row needs is read HERE, once, and passed down.**
  //
  // `TreeNode` used to open nine store subscriptions of its own. At the plugin's
  // own bound of 2,000 rows in one folder that is eighteen thousand selector
  // callbacks on **every** `setState` — and `subscribeToPreview` writes
  // `position` at 30 Hz while a sample auditions, so the tree was running roughly
  // half a million selector calls a second for a value no row reads.
  //
  // ⚠ This is also the shape virtualization needs (TASK-058), so it is not work
  // that gets redone: the rows become props, and the flattening slots in above
  // them rather than inside them.
  const view: TreeView = {
    expanded: useExplorer((s) => s.expanded),
    children: useExplorer((s) => s.children),
    selected: useExplorer((s) => s.selected),
    folder: useExplorer((s) => s.folder),
    truncatedIn: useExplorer((s) => s.truncatedIn),
    starred: useExplorer((s) => s.starred),
    toggleFolder: useExplorer((s) => s.toggleFolder),
    toggleFavourite: useExplorer((s) => s.toggleFavourite),
    select: useExplorer((s) => s.select),
  };

  // ⚠ **Falls back to the first tab rather than showing nothing.** `activeRoot`
  // starts `null` and a removed root leaves it naming a folder that is gone; in
  // both cases the honest answer is the first library folder there is, not an
  // empty panel that looks like a browser with no library.
  const shown = roots.find((root) => samePath(activeRoot, root.path)) ?? roots[0];
  if (shown === undefined) return null;

  return (
    <ul className="tree" role="tree" aria-label={t('explorer.listLabel')}>
      <TreeNode key={shown.path} entry={shown} depth={0} view={view} />
    </ul>
  );
}

/** Everything a row draws from, read once in [`FileTree`]. */
type TreeView = {
  expanded: string[];
  children: Record<string, ExplorerEntry[]>;
  selected: string | null;
  folder: string | null;
  truncatedIn: string[];
  starred: Set<string>;
  toggleFolder: (path: string) => Promise<void>;
  toggleFavourite: (path: string) => Promise<void>;
  select: (path: string) => Promise<void>;
};

/**
 * One row, and — if it is an expanded folder — everything under it.
 *
 * ⚠ **Recursive rather than a flattened list with a depth column.** Flattening
 * is what a virtualized tree needs and this one is not virtualized yet
 * (TASK-058's 2,000-row bound is the plugin's, per folder). When it is, the
 * flattening belongs in the store where it can be memoized, not here — and the
 * props below are already the shape it would hand down.
 */
function TreeNode({
  entry,
  depth,
  view,
}: {
  entry: ExplorerEntry;
  depth: number;
  view: TreeView;
}) {
  const { t } = useTranslation();
  const { children, selected, folder, toggleFolder, toggleFavourite, select } = view;
  // ⚠ A set lookup rather than a scan: `favourites.some(…)` per row is
  // O(rows × favourites), which is a million comparisons at 2,000 × 500.
  const isStarred = view.starred.has(entry.path);

  const isOpen = entry.isDir && view.expanded.some((held) => samePath(held, entry.path));
  const rows = children[entry.path];
  const isTruncated = view.truncatedIn.some((held) => samePath(held, entry.path));
  // ⛔ **Which folder the dice draws from**, marked because it is not
  // necessarily the one you are looking at — expanding a node makes it current,
  // but collapsing deliberately does not hand that back. Without the marker
  // "randomise this pad from the selected folder" is a control with an invisible
  // input.
  const isCurrent = entry.isDir && samePath(folder, entry.path);

  return (
    <li className="tree__node" role="none">
      {/* ⛔⛔ **The star is a SIBLING of the row, not a child of it.** Mike asked
          for it *"to the left of the sample name"*, which reads as "inside the
          row" — but a button inside a button is not a thing HTML allows, and
          `PadGrid`'s `pad__face` records this project already making that mistake
          once: clicks went to whichever control the browser felt like. They sit
          side by side in a flex line instead, so the star *looks* like it is in
          the row and behaves like the separate control it has to be.
          ⚠ **The indent lives on the line**, so the star moves in with its row
          rather than every star lining up in one column regardless of depth. */}
      <div className="tree__line" style={{ paddingInlineStart: `${depth * INDENT + 4}px` }}>
        {/* ⚠ Folders get a spacer, not a star: a favourite is a file you want to
            find again, and finding folders is what the tabs are for. The spacer
            keeps folder and file names in one column. */}
        {entry.isDir ? (
          <span className="tree__star tree__star--spacer" aria-hidden="true" />
        ) : (
          <button
            type="button"
            className="tree__star"
            aria-pressed={isStarred}
            aria-label={t(isStarred ? 'explorer.unstar' : 'explorer.star', {
              name: entry.name,
            })}
            title={t(isStarred ? 'explorer.unstar' : 'explorer.star', { name: entry.name })}
            onClick={() => void toggleFavourite(entry.path)}
          >
            {/* ⛔ **Outline when unstarred, solid when starred, yellow either
                way** — Mike named all three. `fill` rather than a second icon, so
                the glyph does not shift by a pixel as it toggles. */}
            <Star size={11} aria-hidden="true" fill={isStarred ? 'currentColor' : 'none'} />
          </button>
        )}
        <button
          type="button"
          role="treeitem"
          aria-expanded={entry.isDir ? isOpen : undefined}
          aria-selected={!entry.isDir && selected === entry.path}
          className="tree__row"
          data-kind={entry.isDir ? 'dir' : 'file'}
          data-selected={selected === entry.path}
          data-current={isCurrent}
          // ⛔ **The row carries its own path**, so the panel's key handler can
          // act on the row that has *focus* rather than on whatever was last
          // clicked. Those are not the same row: Tab and the arrow keys move
          // focus without selecting, and a producer walking the tree from the
          // keyboard would otherwise keep auditioning the file they left behind.
          data-path={entry.path}
          title={entry.name}
          // ⛔ Files are draggable, folders are not — there is nothing a folder
          // could mean when dropped on one drum lane.
          draggable={!entry.isDir}
          onDragStart={(event) => {
            // ⛔ **`text/plain` as well as the private type.** Some WebView2
            // builds refuse to start a drag whose DataTransfer carries only an
            // unrecognised MIME type, and the drag then silently never begins.
            event.dataTransfer.setData(
              entry.kind === 'midi' ? MIDI_TYPE : SAMPLE_TYPE,
              entry.path,
            );
            event.dataTransfer.setData('text/plain', entry.path);
            event.dataTransfer.effectAllowed = 'copy';
          }}
          onClick={() => void (entry.isDir ? toggleFolder(entry.path) : select(entry.path))}
        >
          {/* ⚠ The twisty is drawn for folders only, and files get a spacer of
              the same width so their names line up with their siblings' rather
              than sitting one glyph to the left. */}
          {entry.isDir ? (
            isOpen ? (
              <ChevronDown className="tree__twisty" size={12} aria-hidden="true" />
            ) : (
              <ChevronRight className="tree__twisty" size={12} aria-hidden="true" />
            )
          ) : (
            <span className="tree__twisty" aria-hidden="true" />
          )}

          {/* ⛔ **The folder icon says open or shut too** — Mike, 2026-08-10:
              *"it should show the folder icons as open or shut depending on if
              they are open or shut as well."* The twisty already carries the
              state, and carrying it twice is the point: the icon is what the eye
              lands on when scanning a column of names, and a chevron 12px wide
              is not what tells you at a glance which branch you left open. */}
          {entry.isDir ? (
            isOpen ? (
              <FolderOpen size={12} aria-hidden="true" />
            ) : (
              <Folder size={12} aria-hidden="true" />
            )
          ) : entry.kind === 'midi' ? (
            // ⛔ **A `.mid` has to look different from a sample.** TASK-058's
            // rule is that the two kinds get two sets of affordances and the
            // panel may not confuse them; the icon is where a producer first
            // sees which they are looking at, before reaching for a control that
            // cannot work.
            // ⚠ Only the MIDI icon is new — a sample keeps the note it has
            // always had, because nothing about samples changed.
            <FileMusic size={12} aria-hidden="true" />
          ) : (
            <Music size={12} aria-hidden="true" />
          )}
          <span className="tree__name">{entry.name}</span>
        </button>
      </div>

      {isOpen && (
        <ul className="tree__children" role="group">
          {/* ⚠ **`explorer.decoding`, not a second key saying the same word.**
              It is already "Reading…" in all eighteen catalogs, and two
              spellings of one string is how two panels come to disagree about it
              in languages nobody on this project reads. */}
          {rows === undefined ? (
            <li className="tree__pending" role="none">
              {t('explorer.decoding')}
            </li>
          ) : rows.length === 0 ? (
            <li className="tree__pending" role="none">
              {t('explorer.empty')}
            </li>
          ) : (
            rows.map((child) => (
              <TreeNode key={child.path} entry={child} depth={depth + 1} view={view} />
            ))
          )}
          {/* ⚠ **Reported, never a silent cut** — the failure `MAX_ENTRIES`
              documents: the producer scrolls, does not find their kick, and
              concludes the browser is broken. Per folder now rather than for the
              panel, because in a tree several folders are listed at once and
              only some of them may have been cut. */}
          {isTruncated && (
            <li className="tree__pending" role="none">
              {t('explorer.truncated')}
            </li>
          )}
        </ul>
      )}
    </li>
  );
}
