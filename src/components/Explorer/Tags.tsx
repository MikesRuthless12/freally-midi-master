import { useId, useMemo, useState } from 'react';

import { Tag, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useExplorer, vocabularyOf } from '../../state/explorer';

/**
 * Every tag in use, sorted — memoised.
 *
 * ⚠ **Memoised, for the reason TASK-158E measured on `lib/fuzzy`**: typing in
 * the filter box re-renders this panel on every keystroke, and walking 500
 * tagged files times twelve tags each per character is the same shape of cost
 * that gate found at 5.5 ms a keystroke over 590 roster entries.
 *
 * ⚠ A hook rather than two copies: both controls below need the same list, and
 * the note above is the kind that gets updated in one copy and not the other.
 */
function useVocabulary(): string[] {
  const tags = useExplorer((s) => s.tags);
  return useMemo(() => vocabularyOf(tags), [tags]);
}

/**
 * Tags on the selected file, and a filter over them — the half of TASK-058C
 * that favourites left behind.
 *
 * ⛔ **Two controls in one file, because they are one idea.** A tag nobody can
 * filter by is a label; a filter with nothing to filter by is an empty row.
 * Splitting them into two components would put the reason for each in the other
 * one's file.
 *
 * ⚠ **Keyed by path**, and `plugin/src/tags.rs` records at length why the
 * roadmap's content-hash is unaffordable here: the tree filter means a row that
 * can be hidden is a row whose tags had to be known, and deciding that by
 * content would hash every file in the folder on the host's editor thread.
 */

/**
 * The chips a producer filters by: one per tag in use, plus the star.
 *
 * ⚠ **Drawn only when there is something to filter by.** A row of no chips under
 * the search box is a control that can only do nothing — the same rule the
 * "use selected" button follows for a `.mid`, and the mood picker for a base
 * with no modes.
 */
export function TagFilter({
  active,
  onlyStarred,
  onToggleTag,
  onToggleStarred,
}: {
  active: string[];
  onlyStarred: boolean;
  onToggleTag: (tag: string) => void;
  onToggleStarred: () => void;
}) {
  const { t } = useTranslation();
  const starred = useExplorer((s) => s.starred);
  const vocabulary = useVocabulary();

  if (vocabulary.length === 0 && starred.size === 0) return null;

  return (
    <div className="browser__tagfilter" role="group" aria-label={t('explorer.tagFilter')}>
      {starred.size > 0 && (
        <button
          type="button"
          className="btn-ghost btn-toggle browser__tagchip"
          aria-pressed={onlyStarred}
          data-on={onlyStarred}
          onClick={onToggleStarred}
        >
          {t('explorer.onlyStarred')}
        </button>
      )}
      {vocabulary.map((tag) => {
        const on = active.includes(tag);
        return (
          <button
            key={tag}
            type="button"
            className="btn-ghost btn-toggle browser__tagchip"
            aria-pressed={on}
            data-on={on}
            onClick={() => onToggleTag(tag)}
          >
            {tag}
          </button>
        );
      })}
    </div>
  );
}

/**
 * The selected file's own tags, editable in place.
 *
 * ⛔ **On the selection rather than on every row**, which is Mike's own shape
 * for this panel: the star is the per-row gesture and the preview strip is where
 * the selected file is described. A text field on two thousand rows is two
 * thousand focus targets between a producer and the next sample.
 */
export function TagRow() {
  const { t } = useTranslation();
  const selected = useExplorer((s) => s.selected);
  const tags = useExplorer((s) => s.tags);
  const setTags = useExplorer((s) => s.setTags);
  const [draft, setDraft] = useState('');
  const listId = useId();
  const vocabulary = useVocabulary();

  if (selected === null) return null;

  const held = tags[selected] ?? [];

  /**
   * The file's tags as of *now*, not as of this render.
   *
   * ⛔⛔ **Both writes send a whole list, so a stale one is a lost update.**
   * Mousedown on a chip's `×` fires blur *before* click: `add()` sent
   * `[...held, tag]` and the remove handler then sent `held.filter(…)` computed
   * from the same render's `held` — which did not contain the tag just typed.
   * Last write won and the tag vanished with no error. Reading the store at the
   * moment of the write is what makes the two orderings agree.
   */
  const nowHeld = () => useExplorer.getState().tags[selected] ?? [];

  const add = () => {
    const tag = draft.trim();
    const current = nowHeld();
    // ⚠ Refused silently rather than with a message: an empty box and a
    // duplicate are both "you already have this", and the chip is on screen
    // saying so. The plugin normalises anyway — this only avoids the round trip.
    if (tag === '' || current.some((on) => on.toLowerCase() === tag.toLowerCase())) {
      setDraft('');
      return;
    }
    setDraft('');
    void setTags(selected, [...current, tag]);
  };

  return (
    <div className="browser__tags">
      <span className="browser__tagslabel">
        <Tag size={11} aria-hidden="true" />
        {t('explorer.tags')}
      </span>

      {held.map((tag) => (
        <span key={tag} className="browser__tagchip browser__tagchip--held">
          {tag}
          <button
            type="button"
            className="btn-ghost browser__tagremove"
            aria-label={t('explorer.tagRemove', { tag })}
            onClick={() =>
              void setTags(
                selected,
                nowHeld().filter((on) => on !== tag),
              )
            }
          >
            <X size={10} aria-hidden="true" />
          </button>
        </span>
      ))}

      {/* ⚠ **A `datalist`, not a bespoke menu.** Completion against tags already
          in use is the whole requirement, and the platform's own control is
          keyboard-accessible in eighteen languages without this file owning a
          listbox. */}
      <input
        type="text"
        className="browser__tagadd"
        list={listId}
        value={draft}
        placeholder={t('explorer.tagAdd')}
        aria-label={t('explorer.tagAdd')}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={add}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault();
            add();
          }
          if (event.key === 'Escape') {
            // ⚠ Stopped, or Escape closes the whole panel with a half-typed tag
            // in it — the same guard the filter box takes.
            event.stopPropagation();
            setDraft('');
          }
        }}
      />
      <datalist id={listId}>
        {vocabulary.map((tag) => (
          <option key={tag} value={tag} />
        ))}
      </datalist>
    </div>
  );
}
