import { useEffect, useLayoutEffect, useRef, useState } from 'react';
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
import { samePath, useExplorer, type ExplorerEntry, type TreeRow } from '../../state/explorer';

/**
 * How far one level indents, in pixels.
 *
 * ⚠ Small on purpose. The rail starts at 280px and a sample library is easily
 * six levels deep — at the 16px a desktop file manager uses, the sixth level
 * would have under half the rail left to write a filename in.
 */
const INDENT = 10;

/**
 * How tall one row is, in pixels.
 *
 * ⛔⛔ **The virtualizer's arithmetic and the CSS must not be able to disagree**,
 * so this number is the source of both: it is written onto the scroller as
 * `--tree-row-h` and `.tree__line` takes its height from that. Measuring a row's
 * own box instead is precisely the bug the pitch lane shipped with — the cell
 * was 2px short of the row-to-row distance, so every gesture drifted. A windowed
 * list makes that failure worse rather than subtler: the offset compounds down
 * the list until the rows sit visibly away from the scrollbar.
 *
 * ⚠ **22, not 20, and the two extra pixels are not slack.** A row is 13px type
 * (~15px line box) plus 2px of block padding each side plus a 1px border — about
 * 21. Pinned at 20 the pitch was still exactly right, so the arithmetic held, but
 * every row overflowed its own slot and the focus rings and selection borders of
 * neighbouring rows clipped into each other.
 */
const ROW_H = 22;

/**
 * How many rows are drawn above and below the visible window.
 *
 * ⚠ Enough that a flick of the wheel does not show blank space before React has
 * rendered, and few enough that the count stays small — the whole point is that
 * a 2,000-row folder costs the same as a 20-row one.
 */
const OVERSCAN = 8;

/**
 * The tree (TASK-058).
 *
 * ⛔⛔ **This replaces a one-folder-at-a-time list, on Mike's instruction**,
 * 2026-08-10: *"you need to show it like a real file explorer does, where the
 * main folder is at the top, and then indents and shows the subfolders
 * underneath and then the files beneath that."*
 *
 * ⛔ **Folders sort above files at every level**, which is `explorer::list`'s
 * doing rather than this component's — one ordering, in the place that reads the
 * directory, so the tree cannot disagree with what the plugin thinks it sent.
 *
 * ⛔⛔ **Virtualized, and flat rather than recursive** (TASK-058: *"a 2,000-file
 * folder under 300 ms"*). It used to be nested `<ul>`s built by a component that
 * called itself, and its own note said what that cost: `MAX_ENTRIES` is 2,000
 * **per folder**, several folders can be open at once, and each row is six
 * elements — so a producer with three big folders expanded was asking React for
 * thirty-odd thousand DOM nodes on every keystroke that touched this store.
 * `flattenTree` produces the lines; this draws the ~30 of them that are on
 * screen and spaces the rest with two blank blocks.
 *
 * ⚠ **The rows arrive as a prop rather than being computed here.**
 * `ExplorerPanel` needs the same list for its ↑/↓ walk — which used to read
 * `.tree__row` out of the DOM, and under virtualization the DOM holds only the
 * window. One list, computed once, and the walk cannot disagree with what is
 * drawn.
 */
export function FileTree({
  rows,
  focused,
  onFocusRow,
}: {
  rows: TreeRow[];
  /** The path of the row with keyboard focus, or `null`. */
  focused: string | null;
  /** Called when a row takes focus by any means, so the walk starts from it. */
  onFocusRow: (path: string | null) => void;
}) {
  const { t } = useTranslation();

  // ⛔⛔ **Every slice a row needs is read HERE, once, and passed down.**
  //
  // `TreeNode` used to open nine store subscriptions of its own. At the plugin's
  // own bound of 2,000 rows in one folder that is eighteen thousand selector
  // callbacks on **every** `setState` — and `subscribeToPreview` writes
  // `position` at 30 Hz while a sample auditions, so the tree was running roughly
  // half a million selector calls a second for a value no row reads.
  const view: TreeView = {
    selected: useExplorer((s) => s.selected),
    folder: useExplorer((s) => s.folder),
    starred: useExplorer((s) => s.starred),
    toggleFolder: useExplorer((s) => s.toggleFolder),
    toggleFavourite: useExplorer((s) => s.toggleFavourite),
    select: useExplorer((s) => s.select),
  };

  const scroller = useRef<HTMLDivElement>(null);
  /** The first row the scroll position reaches — see the `onScroll` note. */
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(0);

  // ⚠ **The viewport height is measured rather than assumed**, because this
  // panel is deliberately resizable in both directions: the rail has a drag
  // handle and the section grows when the roster is collapsed. A fixed guess
  // would draw too few rows in a tall rail — visible as blank space below the
  // last row — and needlessly many in a short one.
  useEffect(() => {
    const box = scroller.current;
    if (box === null) return;
    // ⚠ **From the entry's own `contentRect`, not by reading `clientHeight`.**
    // The callback is already handed the measurement; asking the element again
    // forces a fresh layout, and the rail is *horizontally* draggable — so a
    // width-only drag would pay for a layout per frame for a height that never
    // moved.
    const observer = new ResizeObserver(([entry]) => {
      if (entry !== undefined) setHeight(entry.contentRect.height);
    });
    observer.observe(box);
    setHeight(box.clientHeight);
    return () => observer.disconnect();
  }, []);

  const from = Math.max(0, top - OVERSCAN);
  const to = Math.min(rows.length, top + Math.ceil(height / ROW_H) + OVERSCAN);
  const visible = rows.slice(from, to);

  /**
   * Keep the keyboard's row on screen, and focused.
   *
   * ⛔⛔ **The window is scrolled to the row; the row is NOT added to the
   * window.** Widening the slice to reach it looks like the smaller change and
   * is the bug: the slice is a *range*, so a focus 1,500 rows from the scroll
   * position renders 1,530 rows — the exact cost virtualization exists to
   * remove, arriving through the feature meant to work with it.
   *
   * ▶ **Scrolled arithmetically, because the row may not be mounted.**
   * `scrollIntoView` needs an element and there is not one until the window
   * moves; `ROW_H` is the source of the CSS height, so where row *n* sits is
   * known without asking the DOM. Setting `scrollTop` fires the scroll handler,
   * which re-renders with the row inside the ordinary window — and `top` is a
   * dependency here, so this then runs again and finds it.
   *
   * ⚠ **A layout effect, so focus moves before the browser paints.** In a
   * passive one a frame gets through with the ring still on the row the producer
   * has just left.
   */
  useLayoutEffect(() => {
    const box = scroller.current;
    if (focused === null || box === null) return;

    // ⚠ **Over the mounted rows rather than through a selector.** A path is a
    // Windows path: it carries backslashes, which an attribute selector reads as
    // escapes, and `CSS.escape` does not exist in jsdom. There are ~30 rows in
    // the DOM by construction, so scanning them is both cheaper and has no
    // escaping to get wrong.
    const row = Array.from(box.querySelectorAll<HTMLElement>('.tree__row')).find(
      (candidate) => candidate.dataset.path === focused,
    );
    if (row !== undefined) {
      if (document.activeElement !== row) row.focus({ preventScroll: true });
      return;
    }

    // ⚠ **Only when the row is NOT drawn** — which is the whole reason this
    // scrolls at all, and it is why the O(rows) search below is not on the
    // scroll path: a producer flicking the list never reaches it, because the
    // focused row either is on screen or is not the thing that moved.
    const at = rows.findIndex((held) => held.entry?.path === focused);
    if (at < 0) return;
    // Nearest edge rather than centred: walking a folder with ↓ should scroll a
    // line at a time, not jump the list half a panel on every step.
    const rowTop = at * ROW_H;
    if (rowTop < box.scrollTop) box.scrollTop = rowTop;
    else if (rowTop + ROW_H > box.scrollTop + box.clientHeight) {
      box.scrollTop = rowTop + ROW_H - box.clientHeight;
    }
  }, [focused, rows, top, height]);

  return (
    <div
      className="tree"
      ref={scroller}
      role="tree"
      aria-label={t('explorer.listLabel')}
      style={{ '--tree-row-h': `${ROW_H}px` } as React.CSSProperties}
      // ⚠ **The row the list starts at, not the pixel it starts at.** A scroll
      // fires per frame and the drawn window only changes every `ROW_H` of
      // travel, so storing the pixel re-rendered ~20× more often than the output
      // could differ — each one rebuilding the slice and thirty rows.
      onScroll={(event) => {
        const at = Math.floor(event.currentTarget.scrollTop / ROW_H);
        setTop((was) => (was === at ? was : at));
      }}
    >
      {/* ⛔ **Two spacers rather than absolute positioning.** The rows keep the
          ordinary flow — and with it the `:hover` rules, the focus ring and the
          drag source they already had — while the scrollbar still measures the
          whole list. Positioning every row absolutely would have meant a second
          idea of where a row is, which is what the row-height note above is
          about. */}
      <div style={{ height: `${from * ROW_H}px` }} aria-hidden="true" />
      {visible.map((row) =>
        row.entry === null ? (
          <p
            key={row.key}
            className="tree__pending"
            role="none"
            style={{ paddingInlineStart: `${row.depth * INDENT + 22}px` }}
          >
            {t(row.note ?? '')}
          </p>
        ) : (
          <Row key={row.key} row={row} entry={row.entry} view={view} onFocusRow={onFocusRow} />
        ),
      )}
      <div
        style={{ height: `${Math.max(0, rows.length - to) * ROW_H}px` }}
        aria-hidden="true"
      />
    </div>
  );
}

/** Everything a row draws from, read once in [`FileTree`]. */
type TreeView = {
  selected: string | null;
  folder: string | null;
  starred: Set<string>;
  toggleFolder: (path: string) => Promise<void>;
  toggleFavourite: (path: string) => Promise<void>;
  select: (path: string) => Promise<void>;
};

/** One line of the tree. */
function Row({
  row,
  entry,
  view,
  onFocusRow,
}: {
  row: TreeRow;
  entry: ExplorerEntry;
  view: TreeView;
  onFocusRow: (path: string | null) => void;
}) {
  const { t } = useTranslation();
  const { selected, folder, toggleFolder, toggleFavourite, select } = view;
  // ⚠ A set lookup rather than a scan: `favourites.some(…)` per row is
  // O(rows × favourites), which is a million comparisons at 2,000 × 500.
  const isStarred = view.starred.has(entry.path);
  // ⛔ **Which folder the dice draws from**, marked because it is not
  // necessarily the one you are looking at — expanding a node makes it current,
  // but collapsing deliberately does not hand that back. Without the marker
  // "randomise this pad from the selected folder" is a control with an invisible
  // input.
  const isCurrent = entry.isDir && samePath(folder, entry.path);

  return (
    /* ⛔⛔ **The star is a SIBLING of the row, not a child of it.** Mike asked
       for it *"to the left of the sample name"*, which reads as "inside the
       row" — but a button inside a button is not a thing HTML allows, and
       `PadGrid`'s `pad__face` records this project already making that mistake
       once: clicks went to whichever control the browser felt like. They sit
       side by side in a flex line instead, so the star *looks* like it is in
       the row and behaves like the separate control it has to be.
       ⚠ **The indent lives on the line**, so the star moves in with its row
       rather than every star lining up in one column regardless of depth. */
    /* ⚠ `role="none"` so the `treeitem` inside is exposed as a child of the
       `tree` itself. A flat tree is a legal one — `aria-level` is what carries
       the nesting — but only if nothing unlabelled sits between the two. */
    <div
      className="tree__line"
      role="none"
      style={{ paddingInlineStart: `${row.depth * INDENT + 4}px` }}
    >
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
          aria-label={t(isStarred ? 'explorer.unstar' : 'explorer.star', { name: entry.name })}
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
        aria-expanded={entry.isDir ? row.isOpen : undefined}
        aria-selected={!entry.isDir && selected === entry.path}
        // ⚠ **1-based, and it is what carries the nesting now.** The `<ul>`s
        // that used to say it are gone — a virtualized list cannot be recursive
        // — so the depth has to be stated rather than implied by the markup.
        aria-level={row.depth + 1}
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
        // ⚠ **Focus is reported upward rather than inferred.** The walk needs a
        // starting row, and Tab or a click are as legitimate a way to arrive at
        // one as ↓ is — reading it from `document.activeElement` at keydown time
        // would work too, but only for as long as the focused row is mounted,
        // which under virtualization is not a safe assumption.
        onFocus={() => onFocusRow(entry.path)}
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
          row.isOpen ? (
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
          row.isOpen ? (
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
          <FileMusic size={12} aria-hidden="true" />
        ) : (
          <Music size={12} aria-hidden="true" />
        )}
        <span className="tree__name">{entry.name}</span>
      </button>
    </div>
  );
}
