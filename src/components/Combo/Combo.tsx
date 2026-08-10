import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown } from 'lucide-react';

import './Combo.css';

export type ComboOption = {
  id: string;
  name: string;
  /** Drawn at the trailing edge of the row — a tier, a kind, a count. */
  badge?: string | null;
  /**
   * A command, not a choice — picking it *does* something instead of becoming
   * the value.
   *
   * ⛔⛔ **Nothing may ever land on one by accident.** "Original Workflow" sits
   * at the top of the roster (Mike, 2026-08-09: *"no matter which genre is
   * selected"*), and the moment it did, every fallback in this component chose
   * it: the never-blank display showed it, blur committed it, and Enter on an
   * untouched box opened the style editor — a modal dialog that then swallowed
   * every later click. Six e2e specs failed on exactly that.
   *
   * So an action is reachable only by *deliberately* clicking it or arrowing to
   * it and pressing Enter. It is never a fallback, never the initial highlight,
   * and never what an empty query commits.
   */
  action?: boolean;
};

/**
 * A type-to-autofill combobox that always has something selected.
 *
 * ⛔⛔ **Not a native `<select>`, and that is a defect this project already
 * shipped.** Mike, 2026-08-09, with a screenshot: a `<select>` popup inside
 * WebView2 is drawn by the OS, at OS scale, positioned against the *window*
 * rather than the field — with thirty entries it filled most of the screen and
 * floated away from the control it belonged to. Nothing about a native popup is
 * styleable, so the only fix is to own it. `StyleEditor` owns one for the same
 * reason; this is that pattern factored out before a third copy could drift.
 *
 * ⛔ **You cannot end up with nothing chosen.** Mike's rule for the genre picker:
 * *"ensure that it selects a genre even if you stop typing and you cannot just
 * not pick a genre."* Blur commits the best match for whatever was typed, and if
 * nothing matches it restores the previous value — so every exit path leaves a
 * real selection. That is stronger than "required": a required field is allowed
 * to sit empty and complain, and this is not.
 *
 * ⚠ **Filtering is the caller's**, through `filter`. The roster has aliases and
 * tolerates typos (`lib/fuzzy.ts`), and a second matcher written in here would be
 * a second set of rules to keep in step with it.
 */
export function Combo({
  label,
  options,
  value,
  onChange,
  filter,
  placeholder,
  emptyText,
}: {
  label: string;
  options: ComboOption[];
  value: string | null;
  onChange: (id: string) => void;
  /** Narrow `options` for what has been typed. Defaults to a substring match. */
  filter?: (query: string, options: ComboOption[]) => ComboOption[];
  placeholder?: string;
  /** Shown when the query matches nothing. Omit for controls where that
   *  cannot happen — a fixed list of eight lanes, say. */
  emptyText?: (query: string) => string;
}) {
  const listId = useId();
  // ⛔⛔ **Nothing selected shows the PLACEHOLDER, never a borrowed name.**
  //
  // This briefly fell back to the first option so the box would not look empty —
  // Mike, 2026-08-09: *"you cannot have an empty field for the comboboxes."* It
  // was display-only, and it still lied: the rail read "Drake" while nothing was
  // selected, so everything keyed on a real selection quietly did nothing. The
  // pad grid rendered no pads at all, and Mike found it within a minute.
  //
  // ▶ A placeholder answers the same complaint honestly. "Search an artist…" is
  // a prompt; a name the app has not chosen is a claim. The empty state is
  // deliberate now (2026-08-10: *"ensure that they have to pick an artist before
  // they ever even generate anything, because I LOVE that landing screen"*), so
  // the box has to be able to say so.
  const selected = options.find((option) => option.id === value) ?? null;
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState<string | null>(null);
  // ⛔ Starts on the first real choice, never on an action — see `action` above.
  // Opening the list with the highlight already on "Original Workflow" meant one
  // Enter opened a modal nobody asked for.
  const firstChoice = Math.max(
    0,
    options.findIndex((option) => option.action !== true),
  );
  const [active, setActive] = useState(firstChoice);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLUListElement>(null);
  /** Where the portalled menu sits, in viewport coordinates. */
  const [at, setAt] = useState({ top: 0, left: 0, width: 0 });

  // What the box shows: what is being typed, or the selection when it is not.
  const text = typed ?? selected?.name ?? '';

  const narrowed =
    typed === null || typed.trim() === ''
      ? options
      : filter
        ? filter(typed, options)
        : options.filter((option) => option.name.toLowerCase().includes(typed.toLowerCase()));

  // ⚠ `useLayoutEffect`, so the menu is placed before the browser paints —
  // measuring in a plain effect shows it at 0,0 for one frame, which reads as a
  // flash in the corner of the window.
  useLayoutEffect(() => {
    if (!open) return;
    const field = root.current?.getBoundingClientRect();
    if (field === undefined) return;
    const height = menu.current?.offsetHeight ?? 0;
    // Flip above the field when there is no room below it — a pad combobox opens
    // downward into the piano roll, and one near the bottom of a rail would
    // otherwise run off-screen.
    const below = field.bottom + 2;
    const top =
      below + height > window.innerHeight - 8 ? Math.max(8, field.top - height - 2) : below;
    setAt({ top, left: field.left, width: field.width });
  }, [open, narrowed.length]);

  // ⛔ **Every exit lands on a real selection.** Committing the top match is the
  // "even if you stop typing" half; falling back to `value` is what makes a
  // query matching nothing a no-op rather than a way to end up empty.
  function commit(id?: string) {
    // ⛔⛔ **Nothing typed means nothing changes.** Blur used to take the top of
    // the *unfiltered* list, so simply focusing a combobox and clicking away
    // reselected whatever happened to be first — click the genre box, click
    // "Show all", and UK Drill silently became Trap. A producer's selection is
    // not something a stray focus may edit.
    //
    // ⚠ The "commit the best match on blur" rule is Mike's and still holds — it
    // is about what was **typed** (*"if i start typing 'Trap' and i get to 'Tra'
    // and click off of it, it should automatically select the first one"*).
    // With an untouched box there is no query to resolve, so there is nothing to
    // commit.
    const query = typed?.trim() ?? '';
    const landed =
      id ??
      (query === '' ? value : (narrowed.find((option) => option.action !== true)?.id ?? value));
    if (landed != null && landed !== value) onChange(landed);
    setTyped(null);
    setOpen(false);
    setActive(firstChoice);
  }

  // Clicking away is an exit like any other, so it commits rather than
  // abandoning what was typed.
  useEffect(() => {
    if (!open) return;
    function away(event: MouseEvent) {
      if (root.current?.contains(event.target as Node) === false) commit();
    }
    document.addEventListener('mousedown', away);
    return () => document.removeEventListener('mousedown', away);
  });

  return (
    <div className="combo" ref={root}>
      <div className="combo__field">
        <input
          type="text"
          className="combo__input"
          role="combobox"
          aria-expanded={open}
          aria-controls={listId}
          aria-label={label}
          placeholder={placeholder}
          value={text}
          onChange={(event) => {
            setTyped(event.target.value);
            setActive(0);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => {
            // ⚠ Deferred a tick: a click on a row fires blur first, and
            // committing immediately would close the list before the row's own
            // handler ran, so picking with the mouse would select the *typed*
            // match instead of the one clicked.
            window.setTimeout(() => {
              if (root.current?.contains(document.activeElement) !== true) commit();
            }, 0);
          }}
          onKeyDown={(event) => {
            // ⛔ **Wraps at both ends**, which the search box this replaced did
            // and this did not until an e2e spec caught it. Its comment is the
            // reason and is worth keeping: *"wrapping at the ends, so holding a
            // key never dead-ends."* Clamping makes a held arrow key stop
            // silently, which reads as the control having frozen.
            if (event.key === 'ArrowDown') {
              event.preventDefault();
              setOpen(true);
              setActive((at) => (narrowed.length === 0 ? 0 : (at + 1) % narrowed.length));
            } else if (event.key === 'ArrowUp') {
              event.preventDefault();
              setActive((at) =>
                narrowed.length === 0 ? 0 : (at - 1 + narrowed.length) % narrowed.length,
              );
            } else if (event.key === 'Enter') {
              event.preventDefault();
              commit(narrowed[active]?.id);
            } else if (event.key === 'Escape') {
              event.preventDefault();
              // The one path that abandons what was typed — and it still leaves
              // the previous selection in place.
              setTyped(null);
              setOpen(false);
            }
          }}
        />

        {/* ⛔ The arrow shows the whole list, which is Mike's second way in:
            *"either show the list by clicking the arrow on the right side of it,
            or type and it will autofill."* */}
        <button
          type="button"
          className="combo__arrow"
          tabIndex={-1}
          // ⛔ **Hidden from assistive tech, and that is not a shortcut.** The
          // input beside it already *is* the combobox — it carries the role, the
          // `aria-expanded` state and arrow-key navigation — so this button is a
          // second way to do something already reachable, for the mouse. Giving
          // it the same accessible name as the field made it a duplicate control
          // with a duplicate name: `getByRole('button', { name: /Genres/i })`
          // matched both this and the panel header, and the ambiguity failed a
          // test that had nothing to do with either.
          aria-hidden="true"
          onMouseDown={(event) => {
            // `preventDefault` keeps focus in the input, so opening the list
            // does not fire the blur that would immediately commit and close it.
            event.preventDefault();
            setTyped(null);
            setOpen((was) => !was);
          }}
        >
          <ChevronDown size={14} aria-hidden="true" />
        </button>
      </div>

      {/* ⛔⛔ **Portalled into `document.body`, and this is not a nicety.** Mike,
          2026-08-09: *"the 'Artist/Producer' combobox needs to be above the form,
          not being hidden by another layer so that you have to drag a scrollbar
          to view it."* Every rail panel is `overflow: hidden` — deliberately, so
          a squashed panel clips instead of painting over its neighbour — and an
          absolutely positioned menu inside one is clipped by exactly that. No
          `z-index` can escape an ancestor's overflow; leaving the DOM is the only
          way out. `.drag__lanes` already does this for the same reason.
          ⚠ Positioned against the viewport, measured from the field. */}
      {/* ⛔ **A query that matches nothing has to say so.** An empty box that
          simply vanishes reads as the control breaking, not as "no such artist"
          — the old search box said it and losing that would be a regression the
          redesign smuggled in. Rendered outside the menu because there is no
          menu to put it in. */}
      {open && narrowed.length === 0 && emptyText !== undefined && (
        <p className="combo__empty">{emptyText(typed ?? '')}</p>
      )}

      {open &&
        narrowed.length > 0 &&
        createPortal(
          <ul
            ref={menu}
            className="combo__menu"
            id={listId}
            role="listbox"
            aria-label={label}
            style={{ top: at.top, left: at.left, width: at.width }}
          >
            {narrowed.map((option, index) => (
              <li key={option.id}>
                <button
                  type="button"
                  role="option"
                  // ⛔ **`aria-selected` tracks the HIGHLIGHT, not the stored
                  // value.** In an autocomplete listbox the selected option is
                  // the one the keyboard is on — it is what Enter takes, and it
                  // is what a screen reader announces as you arrow down. Keying
                  // it to the current value instead left the highlight invisible
                  // to assistive tech and to any test that asks the accessible
                  // question rather than reading a class.
                  aria-selected={index === active}
                  data-active={index === active}
                  data-current={option.id === value}
                  onMouseEnter={() => setActive(index)}
                  // ⚠ `onMouseDown`, not `onClick`: the input's blur fires first
                  // and would close the list before a click could land, so the
                  // row picked with the mouse never got chosen.
                  onMouseDown={(event) => {
                    event.preventDefault();
                    commit(option.id);
                  }}
                >
                  <span className="combo__name">{option.name}</span>
                  {option.badge != null && <span className="combo__badge">{option.badge}</span>}
                </button>
              </li>
            ))}
          </ul>,
          document.body,
        )}
    </div>
  );
}
