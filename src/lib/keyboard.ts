/**
 * Shared keyboard rules.
 *
 * ⛔ **This exists because the same predicate was written three times and the
 * copies had already drifted.** `App.tsx` tests
 * `'input, textarea, select, [contenteditable]'` for the `K` shortcut and
 * `'input, textarea, [contenteditable]'` for undo — the second silently lets
 * `Ctrl`+`Z` be stolen from a `<select>` — and the piano roll's shortcuts were
 * about to add a third. What varies between those handlers is which *modifiers*
 * they care about; "is the user typing" does not vary, and should not be
 * restated by each of them.
 */

/**
 * Is this event heading for something the user is typing into?
 *
 * A shortcut must never take a key out of a text field: `Delete` there deletes a
 * character, `Ctrl`+`A` selects the text, and `Ctrl`+`Z` un-types rather than
 * stepping the session back.
 *
 * `closest` rather than `matches`, because a `contenteditable` region can have
 * arbitrary markup inside it and the event target is whatever leaf was clicked
 * — matching only the element itself misses every nested one.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (target === null || !(target instanceof Element)) return false;
  return target.closest('input, textarea, select, [contenteditable]') !== null;
}
